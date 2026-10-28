//! File logging (`<data>/logs/`, daily rotation, 7 days kept). Never writes to the terminal.

use std::path::Path;
use std::str::FromStr;

use tracing::level_filters::LevelFilter;
use tracing_appender::rolling::{Builder, RollingFileAppender, Rotation};

use crate::env::Env;

/// Number of daily log files kept.
const KEEP_DAYS: usize = 7;

/// Creates the rolling appender, or `None` when the folder cannot be used.
fn build_appender(logs_dir: &Path) -> Option<RollingFileAppender> {
    // The appender prunes old files at start-up and prints to stderr if the folder is missing.
    std::fs::create_dir_all(logs_dir).ok()?;
    Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("tokred")
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .build(logs_dir)
        .ok()
}

/// Starts logging into `logs_dir`. Failure to do so is ignored: logging is best effort.
pub fn init(logs_dir: &Path) {
    let level = std::env::var("TOKRED_LOG")
        .ok()
        .and_then(|v| LevelFilter::from_str(&v).ok())
        .unwrap_or(LevelFilter::INFO);
    let Some(appender) = build_appender(logs_dir) else {
        return;
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(appender)
        .with_ansi(false)
        .with_max_level(level)
        .try_init();
}

/// Logging for `tokred hook`: only when the data folder already exists, so that a hook never
/// recreates a folder the user purged.
pub fn init_for_hook() {
    if let Ok(env) = Env::from_process()
        && env.data.root().is_dir()
    {
        init(&env.data.logs_dir());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_appender_creates_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join("a").join("logs");
        assert!(build_appender(&logs).is_some());
        assert!(logs.is_dir());
    }

    #[test]
    fn an_unusable_folder_means_no_logging_and_no_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a file");
        std::fs::write(&file, "x").unwrap();
        assert!(build_appender(&file.join("logs")).is_none());
        // `init` returns quietly (and installs no global subscriber) in that case.
        init(&file.join("logs"));
    }
}
