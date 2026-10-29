//! Process environment: where things live on this machine.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use tokred_core::DataLayout;
use tokred_core::paths::{home_dir, platform_data_dir, resolve_data_dir};

/// Non-empty value of an environment variable.
fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v: &OsString| !v.is_empty())
        .map(PathBuf::from)
}

/// Resolved locations.
#[derive(Debug, Clone)]
pub struct Env {
    /// `$CLAUDE_CONFIG_DIR`, if set.
    pub claude_config_dir: Option<PathBuf>,
    /// `<home>/.claude`.
    pub default_claude_dir: Option<PathBuf>,
    /// The user's home folder.
    pub home_dir: Option<PathBuf>,
    /// tokred data folder.
    pub data: DataLayout,
    /// `$ANTHROPIC_BASE_URL` as exported to this process (empty counts as unset). Read once here
    /// and not where it is used, so a test never depends on the environment of whoever runs it.
    pub anthropic_base_url: Option<String>,
}

impl Env {
    /// Reads the environment of the current process.
    pub fn from_process() -> Result<Self> {
        let base = BaseDirs::new();
        let windows = cfg!(windows);
        let platform_default = platform_data_dir(
            windows,
            var("APPDATA"),
            base.as_ref().map(|b| b.data_dir().to_path_buf()),
        )
        .context("could not determine the data directory; set TOKRED_DATA_DIR")?;
        let home = home_dir(
            windows,
            var("USERPROFILE"),
            base.as_ref().map(|b| b.home_dir().to_path_buf()),
        );
        Ok(Self {
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
            default_claude_dir: home.as_ref().map(|h| h.join(".claude")),
            home_dir: home,
            data: DataLayout::new(resolve_data_dir(var("TOKRED_DATA_DIR"), platform_default)),
            anthropic_base_url: std::env::var("ANTHROPIC_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        })
    }
}
