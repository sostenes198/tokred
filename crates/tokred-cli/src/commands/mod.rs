//! Command implementations: composition of adapters and use cases, plus terminal output.

mod audit;
mod configure;
mod doctor;
mod housekeeping;
mod pricing;
mod unconfigure;
mod update;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokred_core::paths::{claude_dir_candidates, is_valid_claude_dir};
use tokred_core::{Clock, Config, FileSystem, Timestamp};
use tokred_install::StdFileSystem;
use tokred_pricing::http::{LITELLM_URL, LiteLlmSource};
use tokred_store::Store;
use tokred_update::http::{GithubReleaseSource, LATEST_RELEASE_URL, UreqDownloader};
use tokred_update::{StdBinarySwap, current_target};

use crate::args::{Cli, Command};
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
        Command::Configure { dir, yes, force } => {
            configure::run(env, io, dir.claude_dir, yes, force)
        }
        Command::Unconfigure { dir, purge } => unconfigure::run(env, io, dir.claude_dir, purge),
        Command::Doctor {
            dir,
            json,
            bench: _,
        } => doctor::run(env, io, dir.claude_dir, json),
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
        // Wired once the hook storage lands.
        Command::Stats { .. } | Command::Metrics { .. } | Command::Cache { .. } => {
            bail!("this command is not available yet")
        }
    }
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

    /// A sandbox: a Claude folder (with a space in its name) and a data folder.
    pub struct Sandbox {
        pub root: tempfile::TempDir,
        pub claude: PathBuf,
        pub env: Env,
    }

    impl Sandbox {
        pub fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
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
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

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
