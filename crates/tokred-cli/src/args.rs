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

/// Parses a yes/no answer given as a flag or environment value: `true`, `false`, `yes`, `no`,
/// `1` or `0`, in any letter case.
pub fn parse_switch(text: &str) -> Result<bool, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Ok(true),
        "false" | "no" | "0" => Ok(false),
        _ => Err(format!(
            "`{text}` is not a yes/no value (use true, false, yes, no, 1 or 0)"
        )),
    }
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
    /// One row per request origin (main, subagent, auxiliary, compaction).
    Origin,
    /// One row per agent type of a subagent request.
    AgentType,
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
    /// Restart the background proxy (start it when it is not running), to apply a changed setting.
    Restart,
    /// Turn the proxy features on: choose the port and whether conversation content is recorded
    /// (asked in a terminal), save them, start the proxy and point Claude Code's settings at it.
    Enable {
        /// Port the proxy listens on (1024-65535; default: the saved one, else 1029). Also
        /// `TOKRED_PORT`.
        #[arg(long, value_name = "PORT", value_parser = crate::commands::port::parse_port)]
        port: Option<u16>,
        /// Let the dashboard show what was sent and returned: true, false, yes, no, 1 or 0
        /// (kept on this machine only). Also `TOKRED_RECORD_CONTENT`.
        #[arg(long, value_name = "BOOL", value_parser = parse_switch)]
        record_content: Option<bool>,
        /// Do not ask anything: use the values given, the saved ones or the defaults.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Turn the proxy features off (`[proxy] enabled = false`) and remove tokred's
    /// `ANTHROPIC_BASE_URL` from Claude Code's settings.
    Disable,
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
    /// Mode, what routing did (or would do) over a period, and recent decisions.
    Status {
        /// How far back to look (`12h`, `7d`, `2w`, `30d`).
        #[arg(long, default_value = "7d")]
        since: String,
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
    // `--version` here is the version to install, not the program's own version flag.
    #[command(disable_version_flag = true)]
    Update {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// Install exactly this version (a release candidate like `0.2.0-rc.1`, or an older
        /// one) instead of the latest stable release.
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
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
    /// Remove tokred from this machine completely, without asking: the proxy and its service,
    /// the hooks, agents and proxy URL in Claude Code, the data folder, the binary, the PATH
    /// entry and the `claude` wrapper.
    Uninstall {
        /// Folder selection.
        #[command(flatten)]
        dir: ClaudeDirArg,
        /// Accepted and ignored (nothing is asked any more).
        #[arg(long, short = 'y', hide = true)]
        yes: bool,
        /// Accepted and ignored (the data folder is always deleted).
        #[arg(long, hide = true)]
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
    /// Make Claude Code's settings agree with `[proxy] enabled` (run by the `claude` shell
    /// wrapper before every launch; never fails, quiet unless it has something to say).
    Initialize {
        /// The default: print only warnings (accepted so the wrapper can say it explicitly).
        #[arg(long, conflicts_with = "verbose")]
        quiet: bool,
        /// Also say what was changed.
        #[arg(long, short)]
        verbose: bool,
        /// Machine-readable report on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Open the proxy's live dashboard (sessions, what was sent and answered) in the browser.
    Dashboard {
        /// Print the address instead of opening the browser.
        #[arg(long)]
        no_open: bool,
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
        for (flag, expected) in [
            ("origin", StatsBy::Origin),
            ("agent-type", StatsBy::AgentType),
        ] {
            assert!(matches!(
                parse(&["stats", "--by", flag]),
                Command::Stats { command: None, ref args, .. } if args.by == Some(expected)
            ));
        }
        assert!(matches!(
            parse(&["stats", "latency", "--by", "session"]),
            Command::Stats { command: Some(StatsCommand::Latency(ref latency)), .. }
                if latency.by == Some(LatencyBy::Session) && latency.since == "7d"
        ));
    }

    #[test]
    fn configure_takes_no_proxy_options() {
        assert!(matches!(
            parse(&["configure", "--yes"]),
            Command::Configure { yes: true, .. }
        ));
        for flag in [
            ["configure", "--proxy-port", "2000"],
            ["configure", "--record-content", "true"],
            ["configure", "--no-record-content", "x"],
        ] {
            assert!(Cli::try_parse_from(["tokred"].into_iter().chain(flag)).is_err());
        }
    }

    #[test]
    fn proxy_enable_takes_its_answers_as_values() {
        assert!(matches!(
            parse(&["proxy", "enable"]),
            Command::Proxy {
                command: ProxyCommand::Enable {
                    port: None,
                    record_content: None,
                    yes: false
                }
            }
        ));
        assert!(matches!(
            parse(&[
                "proxy",
                "enable",
                "--port",
                "1050",
                "--record-content",
                "Yes"
            ]),
            Command::Proxy {
                command: ProxyCommand::Enable {
                    port: Some(1050),
                    record_content: Some(true),
                    yes: false
                }
            }
        ));
        assert!(matches!(
            parse(&["proxy", "enable", "--record-content=0"]),
            Command::Proxy {
                command: ProxyCommand::Enable {
                    record_content: Some(false),
                    ..
                }
            }
        ));
        for flag in ["--yes", "-y"] {
            assert!(matches!(
                parse(&["proxy", "enable", flag]),
                Command::Proxy {
                    command: ProxyCommand::Enable { yes: true, .. }
                }
            ));
        }
        for bad in [
            ["--port", "80"],
            ["--port", "99999"],
            ["--port", "abc"],
            ["--record-content", "maybe"],
        ] {
            let all = ["tokred", "proxy", "enable"].into_iter().chain(bad);
            assert!(Cli::try_parse_from(all).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn yes_no_values_accept_the_usual_spellings() {
        for yes in ["true", "TRUE", "yes", "Yes", "1", " yes\n"] {
            assert_eq!(parse_switch(yes), Ok(true), "{yes}");
        }
        for no in ["false", "No", "NO", "0"] {
            assert_eq!(parse_switch(no), Ok(false), "{no}");
        }
        let message = parse_switch("maybe").unwrap_err();
        assert!(
            message.contains("`maybe` is not a yes/no value"),
            "{message}"
        );
    }

    #[test]
    fn uninstall_parses() {
        assert!(matches!(parse(&["uninstall"]), Command::Uninstall { .. }));
        // The old options still parse (and do nothing), so an old command does not fail.
        assert!(matches!(
            parse(&["uninstall", "-y", "--purge", "--claude-dir", "/c"]),
            Command::Uninstall { .. }
        ));
    }

    #[test]
    fn the_other_commands_parse() {
        assert!(matches!(
            parse(&["update"]),
            Command::Update { version: None, .. }
        ));
        assert!(matches!(
            parse(&["update", "--version", "0.2.0-rc.1"]),
            Command::Update { version: Some(ref v), .. } if v == "0.2.0-rc.1"
        ));
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
    fn the_dashboard_command_parses() {
        assert!(matches!(
            parse(&["dashboard"]),
            Command::Dashboard { no_open: false }
        ));
        assert!(matches!(
            parse(&["dashboard", "--no-open"]),
            Command::Dashboard { no_open: true }
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
            parse(&["proxy", "disable"]),
            Command::Proxy {
                command: ProxyCommand::Disable
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
    fn initialize_is_quiet_by_default_and_rejects_contradictions() {
        assert!(matches!(
            parse(&["initialize"]),
            Command::Initialize {
                quiet: false,
                verbose: false,
                json: false
            }
        ));
        assert!(matches!(
            parse(&["initialize", "--quiet"]),
            Command::Initialize { quiet: true, .. }
        ));
        assert!(matches!(
            parse(&["initialize", "--verbose", "--json"]),
            Command::Initialize {
                verbose: true,
                json: true,
                ..
            }
        ));
        assert!(Cli::try_parse_from(["tokred", "initialize", "--quiet", "--verbose"]).is_err());
        assert!(Cli::try_parse_from(["tokred", "initialize", "--nope"]).is_err());
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
            parse(&["routing", "status", "--since", "14d"]),
            Command::Routing {
                command: RoutingCommand::Status { ref since, json: false }
            } if since == "14d"
        ));
        assert!(matches!(
            parse(&["proxy", "restart"]),
            Command::Proxy {
                command: ProxyCommand::Restart
            }
        ));
        assert!(matches!(
            parse(&["routing", "status", "--json"]),
            Command::Routing {
                command: RoutingCommand::Status { json: true, .. }
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
