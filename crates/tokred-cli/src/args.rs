//! Command line definition.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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

/// Grouping of `tokred stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StatsBy {
    /// One row per model.
    Model,
    /// One row per day.
    Day,
    /// One row per project.
    Project,
}

/// Grouping of `tokred stats latency`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LatencyBy {
    /// One row per hook.
    Hook,
    /// One row per day.
    Day,
    /// One row per session.
    Session,
}

/// Options of `tokred stats`.
#[derive(Debug, Args)]
pub struct StatsArgs {
    /// How far back to look (`12h`, `7d`, `2w`, `30d`).
    #[arg(long, default_value = "7d")]
    pub since: String,
    /// Group the rows.
    #[arg(long, value_enum)]
    pub by: Option<StatsBy>,
    /// Machine-readable output.
    #[arg(long)]
    pub json: bool,
}

/// Options of `tokred stats latency`.
#[derive(Debug, Args)]
pub struct LatencyArgs {
    /// How far back to look (`12h`, `7d`, `2w`, `30d`).
    #[arg(long, default_value = "7d")]
    pub since: String,
    /// Group the rows.
    #[arg(long, value_enum)]
    pub by: Option<LatencyBy>,
    /// Machine-readable output.
    #[arg(long)]
    pub json: bool,
}

/// Sub-commands of `tokred stats`.
#[derive(Debug, Subcommand)]
pub enum StatsCommand {
    /// How much time the tokred hooks added to Claude Code.
    Latency(LatencyArgs),
}

/// Sub-commands of `tokred pricing`.
#[derive(Debug, Subcommand)]
pub enum PricingCommand {
    /// Refresh the price table from the public price list.
    Sync,
    /// Show the price table.
    Show {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
}

/// Sub-commands of `tokred cache`.
#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// State of the cache per tier.
    Stats {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Remove expired entries.
    Prune,
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
    /// Update tokred according to how it was installed, then re-apply the configuration.
    Update {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
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
        /// Measure the real latency of every hook (spawn to exit).
        #[arg(long)]
        bench: bool,
    },
    /// Tokens, cost, savings and projection from the Claude Code transcripts.
    #[command(args_conflicts_with_subcommands = true)]
    Stats {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// Sub-command (`latency`); without one, the cost report is shown.
        #[command(subcommand)]
        command: Option<StatsCommand>,
        /// Options of the cost report.
        #[command(flatten)]
        args: StatsArgs,
    },
    /// Generate the visual HTML report and open it in the browser.
    Metrics {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// How far back to look (`12h`, `7d`, `2w`, `30d`).
        #[arg(long, default_value = "30d")]
        since: String,
        /// Only write the file; do not open the browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Estimated weight of the fixed context and MCP servers that are never used.
    Audit {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// Also audit this project's CLAUDE.md and MCP configuration.
        #[arg(long, value_name = "PATH")]
        project: Option<PathBuf>,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Manage the price table.
    Pricing {
        /// What to do.
        #[command(subcommand)]
        command: PricingCommand,
    },
    /// Inspect or clean the tiered cache.
    Cache {
        /// What to do.
        #[command(subcommand)]
        command: CacheCommand,
    },
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::*;

    fn parse(args: &[&str]) -> Command {
        let mut full = vec!["tokred"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).unwrap().command
    }

    #[test]
    fn the_command_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn stats_has_defaults_and_a_latency_subcommand() {
        let Command::Stats {
            command: None,
            args,
            ..
        } = parse(&["stats"])
        else {
            panic!()
        };
        assert_eq!(args.since, "7d");
        assert!(args.by.is_none() && !args.json);

        let Command::Stats {
            command: None,
            args,
            ..
        } = parse(&["stats", "--since", "30d", "--by", "model", "--json"])
        else {
            panic!()
        };
        assert_eq!(
            (args.since.as_str(), args.by, args.json),
            ("30d", Some(StatsBy::Model), true)
        );

        let Command::Stats {
            command: Some(StatsCommand::Latency(latency)),
            ..
        } = parse(&["stats", "latency", "--by", "session"])
        else {
            panic!()
        };
        assert_eq!(latency.by, Some(LatencyBy::Session));
        assert_eq!(latency.since, "7d");
    }

    #[test]
    fn the_other_commands_parse() {
        assert!(matches!(parse(&["update"]), Command::Update { .. }));
        assert!(matches!(
            parse(&["doctor", "--bench", "--json"]),
            Command::Doctor {
                bench: true,
                json: true,
                ..
            }
        ));
        let Command::Metrics { since, no_open, .. } = parse(&["metrics", "--no-open"]) else {
            panic!()
        };
        assert_eq!((since.as_str(), no_open), ("30d", true));
        let Command::Audit { project, json, .. } = parse(&["audit", "--project", "/p", "--json"])
        else {
            panic!()
        };
        assert_eq!((project, json), (Some(PathBuf::from("/p")), true));
        assert!(matches!(
            parse(&["pricing", "sync"]),
            Command::Pricing {
                command: PricingCommand::Sync
            }
        ));
        assert!(matches!(
            parse(&["pricing", "show", "--json"]),
            Command::Pricing {
                command: PricingCommand::Show { json: true }
            }
        ));
        assert!(matches!(
            parse(&["cache", "prune"]),
            Command::Cache {
                command: CacheCommand::Prune
            }
        ));
        assert!(matches!(
            parse(&["cache", "stats"]),
            Command::Cache {
                command: CacheCommand::Stats { json: false }
            }
        ));
    }

    #[test]
    fn unknown_values_are_rejected() {
        assert!(Cli::try_parse_from(["tokred", "stats", "--by", "colour"]).is_err());
        assert!(Cli::try_parse_from(["tokred", "nope"]).is_err());
    }
}
