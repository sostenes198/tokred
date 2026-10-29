//! Command implementations: composition of adapters and use cases, plus terminal output.

mod advice;
mod audit;
mod bench;
mod cache;
mod claude;
mod configure;
mod data;
mod doctor;
mod housekeeping;
mod metrics;
mod port;
mod pricing;
mod proxy;
mod report_data;
mod serve;
mod stats;
mod unconfigure;
mod uninstall;
mod update;

pub use bench::BenchReport;
#[cfg(test)]
pub use bench::HookBench;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use tokred_core::paths::{claude_dir_candidates, is_valid_claude_dir};
use tokred_core::{Clock, Config, FileSystem, Timestamp};
use tokred_install::StdFileSystem;
use tokred_pricing::http::{LITELLM_URL, LiteLlmSource};
use tokred_store::Store;
use tokred_update::http::{GithubReleaseSource, LATEST_RELEASE_URL, UreqDownloader};
use tokred_update::{StdBinarySwap, current_target};

use tokred_report::SystemOpener;

use crate::args::{Cli, Command, ProxyArgs, StatsCommand};
use crate::env::Env;
use crate::io::Io;

use self::housekeeping::{Network, Switches};

/// Runs the parsed command.
pub fn run(cli: Cli, io: &mut Io) -> Result<()> {
    let env = Env::from_process()?;
    let version = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe()
        .ok()
        .map(|path| path.canonicalize().unwrap_or(path));
    if let Some(exe) = &exe {
        // A Windows self-update leaves the old executable behind as `.old`.
        tokred_update::cleanup_old_binary(&StdFileSystem, exe);
    }

    let chores = matches!(
        cli.command,
        Command::Doctor { .. }
            | Command::Stats { .. }
            | Command::Metrics { .. }
            | Command::Audit { .. }
            | Command::Pricing { .. }
            | Command::Cache { .. }
    );
    if chores {
        housekeeping::reapply_configuration(&env, io, version);
    }

    let switches = Switches::from_process();
    let prices = LiteLlmSource::new().with_url(url_from_env("TOKRED_PRICES_URL", LITELLM_URL));
    let releases = GithubReleaseSource::new()
        .with_url(url_from_env("TOKRED_RELEASES_URL", LATEST_RELEASE_URL));
    let net = Network {
        prices: &prices,
        releases: &releases,
    };

    let result = dispatch(
        cli.command,
        &env,
        io,
        &net,
        switches,
        exe.as_deref(),
        version,
    );
    if result.is_ok() && chores {
        let exe_text = exe
            .as_deref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        housekeeping::background_chores(&env, io, &net, switches, &exe_text, version);
    }
    result
}

/// The proxy answers for `configure`: a flag wins over `TOKRED_PORT` / `TOKRED_RECORD_CONTENT`
/// (read through `var` so tests need no process environment).
fn proxy_choices(
    args: ProxyArgs,
    var: impl Fn(&str) -> Option<String>,
) -> Result<configure::ProxyChoices> {
    let port = match (
        args.proxy_port,
        var("TOKRED_PORT").filter(|value| !value.is_empty()),
    ) {
        (Some(port), _) => Some(port),
        (None, Some(text)) => {
            Some(port::parse_port(&text).map_err(|error| anyhow!("TOKRED_PORT: {error}"))?)
        }
        (None, None) => None,
    };
    let from_env = var("TOKRED_RECORD_CONTENT")
        .filter(|value| !value.is_empty())
        .map(|value| {
            !matches!(
                value.to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        });
    let record_content = if args.record_content {
        Some(true)
    } else if args.no_record_content {
        Some(false)
    } else {
        from_env
    };
    Ok(configure::ProxyChoices {
        port,
        record_content,
    })
}

/// A URL override from the environment (mirrors, tests), else the built-in one.
fn url_from_env(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn dispatch(
    command: Command,
    env: &Env,
    io: &mut Io,
    net: &Network<'_>,
    switches: Switches,
    exe: Option<&Path>,
    version: &str,
) -> Result<()> {
    match command {
        Command::Configure {
            dir,
            yes,
            force,
            proxy,
        } => {
            let choices = proxy_choices(proxy, |name| std::env::var(name).ok())?;
            let probe = |port| port::probe(&self::proxy::files(env), port);
            let options = configure::Options {
                dir_flag: dir.claude_dir,
                yes,
                force,
                choices,
                probe: &probe,
            };
            configure::run_with(env, io, &options)
        }
        Command::Unconfigure { dir, purge } => unconfigure::run(env, io, dir.claude_dir, purge),
        Command::Uninstall { dir, yes, purge } => {
            let request = uninstall::Request {
                dir_flag: dir.claude_dir,
                yes,
                purge,
            };
            run_uninstall(env, io, exe, request)
        }
        Command::Doctor { dir, json, bench } => {
            let runner = if bench {
                let exe = exe.context("could not locate the running executable")?;
                Some(bench::ProcessRunner { exe })
            } else {
                None
            };
            let runner = runner.as_ref().map(|r| r as &dyn bench::HookRunner);
            doctor::run(env, io, dir.claude_dir, json, runner)
        }
        Command::Audit { dir, project, json } => audit::run(env, io, dir.claude_dir, project, json),
        Command::Pricing { command } => pricing::run(env, io, &command, net),
        Command::Update { dir } => {
            if switches.offline {
                bail!("TOKRED_OFFLINE is set; `tokred update` needs the network");
            }
            let exe = exe.context("could not locate the running executable")?;
            let downloader = UreqDownloader::new();
            let target = current_target();
            let flag = dir.claude_dir;
            let reapply = move |path: &Path| spawn(path, &reapply_args(flag.as_deref()));
            let ctx = update::UpdateContext {
                exe,
                current: version,
                releases: net.releases,
                downloader: &downloader,
                fs: &StdFileSystem,
                swap: &StdBinarySwap,
                target: target.as_deref(),
                reapply: &reapply,
            };
            update::run(io, &ctx)
        }
        Command::Stats { dir, command, args } => match command {
            None => stats::run(env, io, dir.claude_dir, &args),
            Some(StatsCommand::Latency(latency)) => {
                stats::run_latency(env, io, dir.claude_dir, &latency)
            }
        },
        Command::Metrics {
            dir,
            since,
            no_open,
        } => metrics::run(
            env,
            io,
            dir.claude_dir,
            &since,
            no_open,
            &SystemOpener::new(),
        ),
        Command::Cache { command } => cache::run(env, io, &command),
        Command::Proxy { command } => run_proxy(env, io, exe, &command),
        Command::Claude { args } => run_claude(env, io, exe, &args),
        Command::Routing { command } => advice::routing(env, io, &command),
        Command::Compression { command } => run_compression(env, io, &command),
        Command::Suggest { since, json } => advice::suggest(env, io, &since, json),
        Command::Statusline => advice::statusline(env, io),
        Command::Mcp => advice::mcp(env, io, version),
    }
}

/// `tokred proxy ...`: the proxy server runs in this process only for `start --foreground`.
fn run_proxy(
    env: &Env,
    io: &mut Io,
    exe: Option<&Path>,
    command: &crate::args::ProxyCommand,
) -> Result<()> {
    with_proxy_ctx(env, io, exe, |io, ctx| proxy::run(io, ctx, command))
}

/// `tokred uninstall`: the proxy commands plus the rest, with the same adapters.
fn run_uninstall(
    env: &Env,
    io: &mut Io,
    exe: Option<&Path>,
    request: uninstall::Request,
) -> Result<()> {
    with_proxy_ctx(env, io, exe, |io, ctx| uninstall::run(io, ctx, request))
}

/// Builds the real adapters of the proxy commands and hands them to `then`.
fn with_proxy_ctx<T>(
    env: &Env,
    io: &mut Io,
    exe: Option<&Path>,
    then: impl FnOnce(&mut Io, &proxy::ProxyCtx<'_>) -> Result<T>,
) -> Result<T> {
    let exe = exe.context("could not locate the running executable")?;
    let config = housekeeping::load_config(env);
    let claude_dir = select_claude_dir(&StdFileSystem, env, io, None, false).ok();
    let ctx = proxy::ProxyCtx {
        env,
        config: &config,
        exe,
        settle: proxy::SETTLE_TIMEOUT,
        serve_foreground: &|bind: &str| serve::run(env, &config, claude_dir.as_deref(), bind),
        run_command: &crate::daemon::run_planned,
    };
    then(io, &ctx)
}

/// `tokred claude ...`: Claude Code through the proxy for this run, with `claude`'s exit code.
fn run_claude(env: &Env, io: &mut Io, exe: Option<&Path>, args: &[OsString]) -> Result<()> {
    let exe = exe.context("could not locate the running executable")?;
    let config = housekeeping::load_config(env);
    let port = crate::daemon::port_of(&config.proxy.bind)?;
    let claude = claude::find_claude()?;
    let probe = std::time::Duration::from_secs(1);
    let start = || -> Result<()> {
        proxy::files(env).secret()?;
        crate::daemon::spawn_detached(exe, &["proxy", "start", "--foreground"]).map(|_| ())
    };
    let launcher = claude::Launcher {
        port,
        existing_base_url: std::env::var("ANTHROPIC_BASE_URL").ok(),
        healthy: &|port| crate::daemon::healthy(port, probe),
        start_proxy: &start,
        wait_healthy: &|port, timeout| {
            crate::daemon::wait_until(timeout, std::time::Duration::from_millis(100), &mut || {
                crate::daemon::healthy(port, probe)
            })
        },
        run_claude: &|env, args| claude::spawn_inherit(&claude, env, args),
    };
    match claude::run(io, &launcher, args)? {
        0 => Ok(()),
        code => Err(ChildExit(code).into()),
    }
}

/// `tokred compression ...`.
fn run_compression(
    env: &Env,
    io: &mut Io,
    command: &crate::args::CompressionCommand,
) -> Result<()> {
    use crate::args::CompressionCommand as C;
    let dir_flag = match command {
        C::On { dir } | C::Off { dir } => dir.claude_dir.clone(),
        C::Status => None,
    };
    let claude_dir = select_claude_dir(&StdFileSystem, env, io, dir_flag, false).ok();
    let binary = crate::binary::find()?;
    advice::compression(env, io, command, claude_dir, &binary.path)
}

/// Arguments to re-apply the configuration with a freshly installed binary.
fn reapply_args(claude_dir: Option<&Path>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["configure".into(), "--yes".into()];
    if let Some(dir) = claude_dir {
        args.push("--claude-dir".into());
        args.push(dir.into());
    }
    args
}

/// Runs `program args...` with inherited stdio and fails unless it exits successfully.
fn spawn(program: &Path, args: &[OsString]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("could not run {}", program.display()))?;
    if !status.success() {
        bail!(
            "`{} {}` failed ({status})",
            program.display(),
            args.join(&OsString::from(" ")).to_string_lossy()
        );
    }
    Ok(())
}

/// A child program (`claude`) ended with a non-zero exit code; tokred exits with the same code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildExit(pub i32);

impl std::fmt::Display for ChildExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the program exited with code {}", self.0)
    }
}

impl std::error::Error for ChildExit {}

/// Wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        Timestamp(secs)
    }
}

/// The folder saved by a previous `configure` in `tokred.toml`, if any.
pub fn saved_claude_dir(fs: &dyn FileSystem, env: &Env) -> Result<Option<PathBuf>> {
    let path = env.data.config_path();
    let Some(text) = fs.read_to_string(&path)? else {
        return Ok(None);
    };
    let config = Config::from_toml_str(&text)
        .with_context(|| format!("could not read {}", path.display()))?;
    Ok(config.claude.dir.map(PathBuf::from))
}

/// Finds the Claude Code folder: flag, saved value, `CLAUDE_CONFIG_DIR`, default (spec 5.0).
///
/// An explicit flag must be valid. Without one, the first valid candidate wins; when none is
/// valid and the terminal is interactive, the user is asked for a path.
pub fn select_claude_dir(
    fs: &dyn FileSystem,
    env: &Env,
    io: &mut Io,
    flag: Option<PathBuf>,
    allow_prompt: bool,
) -> Result<PathBuf> {
    if let Some(flag) = flag {
        let dir = std::path::absolute(&flag).unwrap_or(flag);
        if is_valid_claude_dir(fs, &dir) {
            return Ok(dir);
        }
        bail!(
            "`{}` is not a Claude Code folder (it must exist and contain settings.json or projects/)",
            dir.display()
        );
    }
    let saved = saved_claude_dir(fs, env)?;
    let candidates = claude_dir_candidates(
        None,
        saved,
        env.claude_config_dir.clone(),
        env.default_claude_dir.clone(),
    );
    if let Some(found) = candidates.iter().find(|dir| is_valid_claude_dir(fs, dir)) {
        return Ok(found.clone());
    }
    if !(allow_prompt && io.interactive()) {
        bail!("could not find the Claude Code folder; pass --claude-dir <path>");
    }
    loop {
        let Some(answer) = io.ask("Claude Code folder not found. Enter its path: ")? else {
            bail!("no Claude Code folder given");
        };
        if answer.is_empty() {
            continue;
        }
        let dir = std::path::absolute(&answer).unwrap_or(PathBuf::from(&answer));
        if is_valid_claude_dir(fs, &dir) {
            return Ok(dir);
        }
        let message = format!(
            "`{}` is not a Claude Code folder (needs settings.json or projects/).",
            dir.display()
        );
        writeln!(io.out(), "{message}")?;
    }
}

/// Opens the database only if it already exists.
pub fn existing_store(env: &Env) -> Result<Option<Store>> {
    let db = env.data.db_path();
    if !StdFileSystem.exists(&db) {
        return Ok(None);
    }
    open_store(&db).map(Some)
}

/// Opens (creating if needed) the database at `db`.
pub fn open_store(db: &Path) -> Result<Store> {
    Store::open(db).with_context(|| format!("could not open {}", db.display()))
}

/// Short display form of a path.
pub fn show(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
pub mod testing {
    //! Shared fixtures for the command tests.

    use std::path::{Path, PathBuf};

    use tokred_core::DataLayout;

    use crate::env::Env;

    /// A local server that answers every request like a healthy proxy and never stops; returns
    /// its port.
    pub fn fake_health_server() -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = r#"{"ok":true}"#;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        port
    }

    /// A release source with a fixed answer that counts its requests.
    pub struct FakeReleases {
        pub version: &'static str,
        pub calls: std::cell::Cell<u32>,
    }

    impl tokred_core::release::ReleaseSource for FakeReleases {
        fn latest(
            &self,
        ) -> Result<tokred_core::release::Release, tokred_core::pricing::SourceError> {
            self.calls.set(self.calls.get() + 1);
            Ok(tokred_core::release::Release {
                version: self.version.to_owned(),
                url: "https://example.invalid".to_owned(),
            })
        }
    }

    #[test]
    fn the_fake_release_source_counts_requests() {
        use tokred_core::release::ReleaseSource as _;
        let fake = FakeReleases {
            version: "1.2.3",
            calls: std::cell::Cell::new(0),
        };
        assert_eq!(fake.latest().unwrap().version, "1.2.3");
        assert_eq!(fake.calls.get(), 1);
    }

    /// A sandbox: a Claude folder (with a space in its name) and a data folder.
    pub struct Sandbox {
        pub root: tempfile::TempDir,
        pub claude: PathBuf,
        pub env: Env,
    }

    impl Sandbox {
        pub fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(root.path().join("home")).unwrap();
            let claude = root.path().join("my claude");
            std::fs::create_dir_all(claude.join("projects")).unwrap();
            let env = Env {
                claude_config_dir: None,
                default_claude_dir: None,
                home_dir: Some(root.path().join("home")),
                data: DataLayout::new(root.path().join("data")),
            };
            Self { root, claude, env }
        }

        pub fn settings(&self) -> PathBuf {
            self.claude.join("settings.json")
        }

        pub fn write_settings(&self, text: &str) {
            std::fs::write(self.settings(), text).unwrap();
        }

        pub fn agent(&self, name: &str) -> PathBuf {
            self.claude.join("agents").join(name)
        }

        pub fn path(&self, name: &str) -> PathBuf {
            Path::new(self.root.path()).join(name)
        }

        /// Runs `configure` so the database, config and hooks exist.
        pub fn configure(&self) {
            let (mut io, _) = crate::io::testing::scripted(false, "");
            super::configure::run(&self.env, &mut io, Some(self.claude.clone()), true, false)
                .unwrap();
        }

        /// The database.
        pub fn store(&self) -> tokred_store::Store {
            tokred_store::Store::open(&self.env.data.db_path()).unwrap()
        }

        /// Writes a transcript with two assistant messages from yesterday (opus and sonnet),
        /// one repeated streaming line, one subagent message and some noise.
        pub fn seed_usage(&self) {
            let now = tokred_core::Clock::now(&super::SystemClock).0;
            let day = crate::view::ymd(now - 86_400);
            let line = |id: &str, model: &str, input: u64| {
                format!(
                    r#"{{"type":"assistant","sessionId":"s1","timestamp":"{day}T12:00:00.000Z","requestId":"r-{id}","cwd":"/p","isSidechain":false,"message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{},"cache_creation_input_tokens":100,"cache_read_input_tokens":2000}}}}}}"#,
                    input / 2
                )
            };
            let text = [
                line("m1", "claude-opus-4-1-20250805", 200_000),
                line("m2", "claude-sonnet-4-20250514", 300_000),
                line("m2", "claude-sonnet-4-20250514", 300_000),
                r#"{"type":"user","message":{"content":"hi"}}"#.to_owned(),
                "not json at all".to_owned(),
            ]
            .join("\n");
            let dir = self.claude.join("projects/-tmp-proj");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("s1.jsonl"), format!("{text}\n")).unwrap();
        }

        /// Records a few hook runs from the last days.
        pub fn seed_events(&self) {
            use tokred_core::hook_event::{
                HookEventRecord, HookEventRepo, Outcome, decision, reason,
            };
            let now = tokred_core::Clock::now(&super::SystemClock).0;
            let store = self.store();
            let event = |hook: &str, decision: &str, why: &str, bytes: u64, us: u64, ago: i64| {
                HookEventRecord {
                    ts: tokred_core::Timestamp(now - ago),
                    session_id: "s1".to_owned(),
                    hook: hook.to_owned(),
                    decision: decision.to_owned(),
                    reason: why.to_owned(),
                    bytes_saved: bytes,
                    duration_us: us,
                    outcome: Outcome::Ok,
                }
            };
            for e in [
                event(
                    "guard-read",
                    decision::DENY,
                    reason::REREAD,
                    40_000,
                    3_000,
                    3_600,
                ),
                event("guard-read", decision::ALLOW, "", 0, 2_000, 3_000),
                event(
                    "guard-search",
                    decision::DENY,
                    reason::BROAD_SEARCH,
                    10_000,
                    4_000,
                    2_000,
                ),
                event(
                    "route",
                    "implementation",
                    reason::SUGGESTED,
                    0,
                    5_000,
                    1_000,
                ),
                event(
                    "enforce",
                    decision::ALLOW,
                    reason::AGENT_MATCHES,
                    0,
                    2_500,
                    900,
                ),
                event(
                    "enforce",
                    decision::WARN,
                    reason::AGENT_MISMATCH,
                    0,
                    2_500,
                    800,
                ),
            ] {
                store.insert_event(&e).unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    #[test]
    fn a_child_exit_names_its_code() {
        assert_eq!(ChildExit(3).to_string(), "the program exited with code 3");
    }

    #[test]
    fn system_clock_is_after_2020() {
        assert!(SystemClock.now().0 > 1_577_836_800);
    }

    #[test]
    fn valid_flag_wins_and_invalid_flag_is_an_error() {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");
        let got = select_claude_dir(
            &StdFileSystem,
            &sb.env,
            &mut io,
            Some(sb.claude.clone()),
            false,
        )
        .unwrap();
        assert_eq!(got, sb.claude);

        let err = select_claude_dir(
            &StdFileSystem,
            &sb.env,
            &mut io,
            Some(sb.path("nope")),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not a Claude Code folder"));
    }

    #[test]
    fn candidates_are_tried_in_order_saved_env_default() {
        let mut sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");

        // Only the default is valid.
        sb.env.default_claude_dir = Some(sb.claude.clone());
        sb.env.claude_config_dir = Some(sb.path("missing-env"));
        assert_eq!(
            select_claude_dir(&StdFileSystem, &sb.env, &mut io, None, false).unwrap(),
            sb.claude
        );

        // A saved folder in tokred.toml is considered too (and may be invalid).
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(
            sb.env.data.config_path(),
            format!(
                "[claude]\ndir = {:?}\n",
                sb.path("saved-but-gone").to_string_lossy()
            ),
        )
        .unwrap();
        assert_eq!(
            select_claude_dir(&StdFileSystem, &sb.env, &mut io, None, false).unwrap(),
            sb.claude
        );
        assert_eq!(
            saved_claude_dir(&StdFileSystem, &sb.env).unwrap(),
            Some(sb.path("saved-but-gone"))
        );
    }

    #[test]
    fn no_candidate_without_a_terminal_suggests_the_flag() {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");
        let err = select_claude_dir(&StdFileSystem, &sb.env, &mut io, None, true).unwrap_err();
        assert!(err.to_string().contains("--claude-dir"));
        // Prompting is also off when the command does not allow it.
        let (mut tty, _) = scripted(true, "");
        let err = select_claude_dir(&StdFileSystem, &sb.env, &mut tty, None, false).unwrap_err();
        assert!(err.to_string().contains("--claude-dir"));
    }

    #[test]
    fn the_prompt_skips_blank_lines_and_rejects_invalid_paths_until_a_valid_one() {
        let sb = Sandbox::new();
        let answers = format!("\n{}\n{}\n", sb.path("nope").display(), sb.claude.display());
        let (mut io, out) = scripted(true, &answers);
        let got = select_claude_dir(&StdFileSystem, &sb.env, &mut io, None, true).unwrap();
        assert_eq!(got, sb.claude);
        assert!(
            out.text()
                .contains("is not a Claude Code folder (needs settings.json")
        );
    }

    #[test]
    fn the_prompt_gives_up_at_end_of_input() {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(true, "");
        let err = select_claude_dir(&StdFileSystem, &sb.env, &mut io, None, true).unwrap_err();
        assert!(err.to_string().contains("no Claude Code folder given"));
    }

    #[test]
    fn an_unreadable_saved_config_is_an_error() {
        let sb = Sandbox::new();
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(sb.env.data.config_path(), "[[[").unwrap();
        let err = saved_claude_dir(&StdFileSystem, &sb.env).unwrap_err();
        assert!(err.to_string().contains("could not read"));
        assert_eq!(
            saved_claude_dir(&StdFileSystem, &Sandbox::new().env).unwrap(),
            None
        );
    }

    #[test]
    fn existing_store_is_none_without_a_database_and_errors_on_a_corrupt_one() {
        let sb = Sandbox::new();
        assert!(existing_store(&sb.env).unwrap().is_none());

        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(sb.env.data.db_path(), "this is not a database, just text").unwrap();
        let err = existing_store(&sb.env).err().unwrap();
        assert!(err.to_string().contains("could not open"), "{err}");
    }

    #[test]
    fn url_overrides_come_from_the_environment() {
        assert_eq!(
            url_from_env("TOKRED_TEST_SURELY_UNSET_URL", "https://default"),
            "https://default"
        );
    }

    #[test]
    fn proxy_choices_come_from_flags_then_the_environment() {
        let none = ProxyArgs {
            proxy_port: None,
            record_content: false,
            no_record_content: false,
        };
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        let nothing = proxy_choices(none, env(&[])).unwrap();
        assert_eq!(nothing, configure::ProxyChoices::default());
        let empty = proxy_choices(
            none,
            env(&[("TOKRED_PORT", ""), ("TOKRED_RECORD_CONTENT", "")]),
        );
        assert_eq!(empty.unwrap(), nothing);

        let from_env = proxy_choices(
            none,
            env(&[("TOKRED_PORT", "2222"), ("TOKRED_RECORD_CONTENT", "1")]),
        )
        .unwrap();
        assert_eq!(
            (from_env.port, from_env.record_content),
            (Some(2222), Some(true))
        );
        for off in ["0", "false", "NO", "off"] {
            let pairs: &'static [(&str, &str)] =
                Box::leak(Box::new([("TOKRED_RECORD_CONTENT", off)]));
            let choices = proxy_choices(none, env(pairs)).unwrap();
            assert_eq!(choices.record_content, Some(false), "{off}");
        }

        let flags = ProxyArgs {
            proxy_port: Some(3333),
            record_content: false,
            no_record_content: true,
        };
        let both = env(&[("TOKRED_PORT", "2222"), ("TOKRED_RECORD_CONTENT", "1")]);
        let chosen = proxy_choices(flags, both).unwrap();
        assert_eq!(
            (chosen.port, chosen.record_content),
            (Some(3333), Some(false))
        );
        let yes = ProxyArgs {
            record_content: true,
            no_record_content: false,
            ..flags
        };
        assert_eq!(
            proxy_choices(yes, env(&[])).unwrap().record_content,
            Some(true)
        );

        let err = proxy_choices(none, env(&[("TOKRED_PORT", "80")])).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("TOKRED_PORT: `80` is not a port"),
            "{err}"
        );
    }

    #[test]
    fn reapply_args_pass_the_claude_dir_along() {
        let bare = reapply_args(None);
        assert_eq!(
            bare,
            vec![OsString::from("configure"), OsString::from("--yes")]
        );
        let with = reapply_args(Some(Path::new("/c d")));
        assert_eq!(with.len(), 4);
        assert_eq!(with[3], OsString::from("/c d"));
    }

    #[test]
    fn spawn_succeeds_or_fails_with_the_programs_status() {
        // The test binary itself is a convenient program: `--list` exits 0, a bad flag does not.
        let me = std::env::current_exe().unwrap();
        spawn(&me, &[OsString::from("--list")]).unwrap();
        let err = spawn(&me, &[OsString::from("--definitely-not-a-flag")]).unwrap_err();
        assert!(err.to_string().contains("failed"), "{err}");
        let missing = spawn(Path::new("/no/such/program"), &[]).unwrap_err();
        assert!(missing.to_string().contains("could not run"));
    }

    #[test]
    fn show_prints_the_path() {
        assert_eq!(
            show(Path::new("a/b")),
            Path::new("a/b").display().to_string()
        );
    }
}
