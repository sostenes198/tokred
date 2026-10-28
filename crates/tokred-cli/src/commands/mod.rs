//! Command implementations: composition of adapters and use cases, plus terminal output.

mod configure;
mod doctor;
mod unconfigure;

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokred_core::paths::{claude_dir_candidates, is_valid_claude_dir};
use tokred_core::{Clock, Config, FileSystem, Timestamp};
use tokred_install::StdFileSystem;
use tokred_store::Store;

use crate::args::{Cli, Command};
use crate::env::Env;

/// Runs the parsed command.
pub fn run(cli: Cli) -> Result<()> {
    let env = Env::from_process()?;
    match cli.command {
        Command::Configure { dir, yes, force } => configure::run(&env, dir.claude_dir, yes, force),
        Command::Unconfigure { dir, purge } => unconfigure::run(&env, dir.claude_dir, purge),
        Command::Doctor { dir, json } => doctor::run(&env, dir.claude_dir, json),
    }
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

/// Whether stdin is an interactive terminal.
pub fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// Asks a question on the terminal and returns the trimmed answer (`None` at end of input).
pub fn ask(question: &str) -> Result<Option<String>> {
    print!("{question}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
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
    if !(allow_prompt && interactive()) {
        bail!("could not find the Claude Code folder; pass --claude-dir <path>");
    }
    loop {
        let Some(answer) = ask("Claude Code folder not found. Enter its path: ")? else {
            bail!("no Claude Code folder given");
        };
        if answer.is_empty() {
            continue;
        }
        let dir = std::path::absolute(&answer).unwrap_or_else(|_| PathBuf::from(&answer));
        if is_valid_claude_dir(fs, &dir) {
            return Ok(dir);
        }
        println!(
            "`{}` is not a Claude Code folder (needs settings.json or projects/).",
            dir.display()
        );
    }
}

/// Opens the database only if it already exists.
pub fn existing_store(env: &Env) -> Result<Option<Store>> {
    let db = env.data.db_path();
    if !StdFileSystem.exists(&db) {
        return Ok(None);
    }
    Ok(Some(Store::open(&db).with_context(|| {
        format!("could not open {}", db.display())
    })?))
}

/// Short display form of a path.
pub fn show(path: &Path) -> String {
    path.display().to_string()
}
