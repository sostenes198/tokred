//! Command line definition.

use std::ffi::OsString;
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

/// Sub-commands of `tokred proxy`.
#[derive(Debug, Subcommand)]
pub enum ProxyCommand {
    /// Start the local proxy (in the background unless `--foreground`).
    Start {
        /// Run in this terminal instead of the background (used by the user service).
        #[arg(long)]
        foreground: bool,
        /// Address to listen on (default: `[proxy] bind`, loopback only).
        #[arg(long, value_name = "HOST:PORT")]
        bind: Option<String>,
    },
    /// Stop the background proxy.
    Stop,
    /// Show whether the proxy is running and what it is doing.
    Status {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Install or remove the proxy as a user service that starts at login.
    Service {
        /// What to do.
        #[command(subcommand)]
        command: ServiceCommand,
    },
}

/// Sub-commands of `tokred proxy service`.
#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Install and start the user service.
    Install,
    /// Stop and remove the user service.
    Uninstall,
}

/// Sub-commands of `tokred routing`.
#[derive(Debug, Subcommand)]
pub enum RoutingCommand {
    /// Mode, circuit breaker and recent decisions.
    Status {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Let routing change the model of subagent and auxiliary requests (`enforce`).
    On,
    /// Stop changing models; keep recording what routing would do (`observe`).
    Off,
    /// Explain one decision.
    Explain {
        /// The decision id (from `tokred routing status`).
        id: String,
    },
}

/// Sub-commands of `tokred compression`.
#[derive(Debug, Subcommand)]
pub enum CompressionCommand {
    /// Replace large tool results with a preview and let the model read the rest back
    /// (registers the `tokred_retrieve` MCP server in Claude Code's user config).
    On {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
    },
    /// Go back to measuring only and unregister the MCP server.
    Off {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
    },
    /// Mode, whether the server is registered and whether the circuit breaker tripped.
    Status,
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
    /// Run and manage the local proxy between Claude Code and the Anthropic API.
    Proxy {
        /// What to do.
        #[command(subcommand)]
        command: ProxyCommand,
    },
    /// Start Claude Code through the local proxy, for this run only.
    Claude {
        /// Arguments passed to `claude` unchanged.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Model routing for subagents and auxiliary requests (the main loop is never changed).
    Routing {
        /// What to do.
        #[command(subcommand)]
        command: RoutingCommand,
    },
    /// Reversible compression of large tool results (off by default: measures only).
    Compression {
        /// What to do.
        #[command(subcommand)]
        command: CompressionCommand,
    },
    /// Cheaper-model suggestions for the main loop, with the evidence.
    Suggest {
        /// How far back to look (`12h`, `7d`, `2w`, `30d`).
        #[arg(long, default_value = "7d")]
        since: String,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// One short line for Claude Code's status line (reads the status-line JSON on stdin).
    Statusline,
    /// Internal: MCP server with the `tokred_retrieve` tool, started by Claude Code.
    #[command(hide = true)]
    Mcp,
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
        assert!(matches!(
            parse(&["stats"]),
            Command::Stats { command: None, ref args, .. }
                if args.since == "7d" && args.by.is_none() && !args.json
        ));
        assert!(matches!(
            parse(&["stats", "--since", "30d", "--by", "model", "--json"]),
            Command::Stats { command: None, ref args, .. }
                if args.since == "30d" && args.by == Some(StatsBy::Model) && args.json
        ));
        assert!(matches!(
            parse(&["stats", "latency", "--by", "session"]),
            Command::Stats { command: Some(StatsCommand::Latency(ref latency)), .. }
                if latency.by == Some(LatencyBy::Session) && latency.since == "7d"
        ));
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
        assert!(matches!(
            parse(&["metrics", "--no-open"]),
            Command::Metrics { ref since, no_open: true, .. } if since == "30d"
        ));
        assert!(matches!(
            parse(&["audit", "--project", "/p", "--json"]),
            Command::Audit { project: Some(ref p), json: true, .. } if p == &PathBuf::from("/p")
        ));
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
    fn the_proxy_commands_parse() {
        assert!(matches!(
            parse(&["proxy", "start", "--foreground", "--bind", "127.0.0.1:9000"]),
            Command::Proxy {
                command: ProxyCommand::Start { foreground: true, bind: Some(ref b) }
            } if b == "127.0.0.1:9000"
        ));
        assert!(matches!(
            parse(&["proxy", "start"]),
            Command::Proxy {
                command: ProxyCommand::Start {
                    foreground: false,
                    bind: None
                }
            }
        ));
        assert!(matches!(
            parse(&["proxy", "stop"]),
            Command::Proxy {
                command: ProxyCommand::Stop
            }
        ));
        assert!(matches!(
            parse(&["proxy", "status", "--json"]),
            Command::Proxy {
                command: ProxyCommand::Status { json: true }
            }
        ));
        assert!(matches!(
            parse(&["proxy", "service", "install"]),
            Command::Proxy {
                command: ProxyCommand::Service {
                    command: ServiceCommand::Install
                }
            }
        ));
        assert!(matches!(
            parse(&["proxy", "service", "uninstall"]),
            Command::Proxy {
                command: ProxyCommand::Service {
                    command: ServiceCommand::Uninstall
                }
            }
        ));
    }

    #[test]
    fn claude_passes_every_argument_through() {
        assert!(matches!(
            parse(&["claude", "--model", "opus", "-p", "hi"]),
            Command::Claude { ref args } if args == &["--model", "opus", "-p", "hi"]
        ));
        assert!(matches!(parse(&["claude"]), Command::Claude { ref args } if args.is_empty()));
    }

    #[test]
    fn routing_suggest_statusline_and_mcp_parse() {
        assert!(matches!(
            parse(&["routing", "status", "--json"]),
            Command::Routing {
                command: RoutingCommand::Status { json: true }
            }
        ));
        assert!(matches!(
            parse(&["routing", "on"]),
            Command::Routing {
                command: RoutingCommand::On
            }
        ));
        assert!(matches!(
            parse(&["routing", "off"]),
            Command::Routing {
                command: RoutingCommand::Off
            }
        ));
        assert!(matches!(
            parse(&["routing", "explain", "d1"]),
            Command::Routing { command: RoutingCommand::Explain { ref id } } if id == "d1"
        ));
        assert!(matches!(
            parse(&["suggest"]),
            Command::Suggest { ref since, json: false } if since == "7d"
        ));
        assert!(matches!(parse(&["statusline"]), Command::Statusline));
        assert!(matches!(parse(&["mcp"]), Command::Mcp));
    }

    #[test]
    fn the_compression_commands_parse() {
        assert!(matches!(
            parse(&["compression", "on"]),
            Command::Compression {
                command: CompressionCommand::On { .. }
            }
        ));
        assert!(matches!(
            parse(&["compression", "off", "--claude-dir", "/c"]),
            Command::Compression {
                command: CompressionCommand::Off { .. }
            }
        ));
        assert!(matches!(
            parse(&["compression", "status"]),
            Command::Compression {
                command: CompressionCommand::Status
            }
        ));
    }

    #[test]
    fn unknown_values_are_rejected() {
        assert!(Cli::try_parse_from(["tokred", "stats", "--by", "colour"]).is_err());
        assert!(Cli::try_parse_from(["tokred", "nope"]).is_err());
    }
}
