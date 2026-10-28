//! Command line definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Token and cost reducer for Claude Code: model routing, context guards and local metrics.
#[derive(Debug, Parser)]
#[command(name = "tokred", version, about, propagate_version = true)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Options shared by the commands that act on a Claude Code folder.
#[derive(Debug, Args)]
pub struct ClaudeDirArg {
    /// Claude Code folder (default: saved value, `$CLAUDE_CONFIG_DIR`, then `~/.claude`).
    #[arg(long, value_name = "PATH", global = true)]
    pub claude_dir: Option<PathBuf>,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Integrate tokred with Claude Code: hooks in settings.json and the tokred subagents.
    Configure {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// Apply without asking for confirmation.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Overwrite agent files even if you edited them.
        #[arg(long)]
        force: bool,
    },
    /// Remove tokred's hooks and agents from Claude Code.
    Unconfigure {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// Also delete tokred's data folder (database, config, logs, reports).
        #[arg(long)]
        purge: bool,
    },
    /// Show what is installed and what needs attention.
    Doctor {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
}
