//! Process environment: where things live on this machine.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use tokred_core::DataLayout;
use tokred_core::paths::resolve_data_dir;

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
    /// tokred data folder.
    pub data: DataLayout,
}

impl Env {
    /// Reads the environment of the current process.
    pub fn from_process() -> Result<Self> {
        let base = BaseDirs::new();
        let platform_default = base
            .as_ref()
            .map(|b| b.data_dir().join("tokred"))
            .context("could not determine the home directory; set TOKRED_DATA_DIR")?;
        Ok(Self {
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
            default_claude_dir: base.as_ref().map(|b| b.home_dir().join(".claude")),
            data: DataLayout::new(resolve_data_dir(var("TOKRED_DATA_DIR"), platform_default)),
        })
    }
}
