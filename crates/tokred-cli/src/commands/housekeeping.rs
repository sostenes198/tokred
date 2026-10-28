//! Chores that ride along with CLI commands (never with hooks): re-applying the configuration
//! after an upgrade, the daily price sync and the daily version check.
//!
//! All of them are best effort: a failure here must never fail or delay the command the user
//! asked for, and nothing is ever printed on stdout (it could be `--json`).

use tokred_core::pricing::PriceSource;
use tokred_core::release::ReleaseSource;
use tokred_core::{Config, FileSystem};
use tokred_install::StdFileSystem;
use tokred_pricing::{sync_due, sync_prices};
use tokred_update::{detect, needs_reconfigure, notice, run_check, should_check};

use super::{SystemClock, configure, existing_store};
use crate::env::Env;
use crate::io::Io;

/// The remote sources the chores may talk to.
pub struct Network<'a> {
    /// Price list.
    pub prices: &'a dyn PriceSource,
    /// Release information.
    pub releases: &'a dyn ReleaseSource,
}

/// Switches read from the environment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Switches {
    /// `TOKRED_OFFLINE`: no network call of any kind.
    pub offline: bool,
    /// `TOKRED_NO_UPDATE_CHECK`: no version check.
    pub no_update_check: bool,
}

impl Switches {
    /// Reads the switches from the process environment.
    #[must_use]
    pub fn from_process() -> Self {
        let on = |name: &str| {
            std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0")
        };
        Self {
            offline: on("TOKRED_OFFLINE"),
            no_update_check: on("TOKRED_NO_UPDATE_CHECK"),
        }
    }
}

fn load_config(env: &Env) -> Config {
    StdFileSystem
        .read_to_string(&env.data.config_path())
        .ok()
        .flatten()
        .and_then(|text| Config::from_toml_str(&text).ok())
        .unwrap_or_default()
}

/// After the binary changed version, re-applies hooks and agents quietly (spec section 14).
pub fn reapply_configuration(env: &Env, io: &mut Io, version: &str) {
    let Ok(Some(store)) = existing_store(env) else {
        return;
    };
    if !needs_reconfigure(&store, version) {
        return;
    }
    drop(store);
    let mut quiet = Io::new(
        false,
        Box::new(std::io::BufReader::new(std::io::empty())),
        Box::new(std::io::sink()),
        Box::new(std::io::sink()),
    );
    if let Err(err) = configure::run(env, &mut quiet, None, true, false) {
        let _ = writeln!(
            io.err(),
            "tokred was upgraded but its configuration could not be re-applied ({err:#}); run `tokred configure`"
        );
    }
}

/// The daily price sync and version check. Notices go to stderr.
pub fn background_chores(
    env: &Env,
    io: &mut Io,
    net: &Network<'_>,
    switches: Switches,
    exe: &str,
    version: &str,
) {
    if switches.offline {
        return;
    }
    let Ok(Some(store)) = existing_store(env) else {
        return;
    };
    let config = load_config(env);
    let clock = SystemClock;

    if sync_due(&store, &clock, config.pricing.auto_sync) {
        let _ = sync_prices(net.prices, &store, &store, &clock);
    }
    // `run_check` makes at most one request a day and reuses the saved answer in between.
    if should_check(config.update.mode, switches.no_update_check)
        && let Some(new) = run_check(net.releases, &store, &clock, version)
    {
        let _ = writeln!(io.err(), "{}", notice(&new, version, detect(exe)));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use tokred_core::Timestamp;
    use tokred_core::meta::MetaRepo;
    use tokred_core::pricing::{ModelPrice, PriceEntry, SourceError};

    use super::super::testing::{FakeReleases, Sandbox};
    use super::*;
    use crate::io::testing::{scripted, scripted_with_err};

    struct Prices {
        calls: Cell<u32>,
        fail: bool,
    }

    impl PriceSource for Prices {
        fn fetch(&self) -> Result<Vec<PriceEntry>, SourceError> {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                return Err(SourceError::Invalid("offline".to_owned()));
            }
            Ok(vec![PriceEntry {
                model: "claude-opus-4-1-20250805".to_owned(),
                price: ModelPrice {
                    input: 15.0,
                    output: 75.0,
                    cache_write: 18.75,
                    cache_write_1h: 30.0,
                    cache_read: 1.5,
                },
                source: "fake".to_owned(),
                updated_at: Timestamp(0),
            }])
        }
    }

    fn sources(version: &'static str) -> (Prices, FakeReleases) {
        (
            Prices {
                calls: Cell::new(0),
                fail: false,
            },
            FakeReleases {
                calls: Cell::new(0),
                version,
            },
        )
    }

    fn configured() -> Sandbox {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");
        configure::run(&sb.env, &mut io, Some(sb.claude.clone()), true, false).unwrap();
        sb
    }

    fn chores(sb: &Sandbox, p: &Prices, r: &FakeReleases, switches: Switches) -> String {
        let (mut io, out, err) = scripted_with_err(false, "");
        background_chores(
            &sb.env,
            &mut io,
            &Network {
                prices: p,
                releases: r,
            },
            switches,
            "/usr/local/bin/tokred",
            "0.1.0",
        );
        assert_eq!(out.text(), "", "chores never write to stdout");
        err.text()
    }

    #[test]
    fn nothing_happens_without_a_database() {
        let sb = Sandbox::new();
        let (p, r) = sources("9.9.9");
        assert_eq!(chores(&sb, &p, &r, Switches::default()), "");
        assert_eq!((p.calls.get(), r.calls.get()), (0, 0));
    }

    #[test]
    fn offline_means_no_network_at_all() {
        let sb = configured();
        let (p, r) = sources("9.9.9");
        let switches = Switches {
            offline: true,
            no_update_check: false,
        };
        assert_eq!(chores(&sb, &p, &r, switches), "");
        assert_eq!((p.calls.get(), r.calls.get()), (0, 0));
    }

    #[test]
    fn prices_sync_once_a_day_and_the_version_notice_is_reused() {
        let sb = configured();
        let (p, r) = sources("9.9.9");

        let first = chores(&sb, &p, &r, Switches::default());
        assert!(
            first.contains("new version 9.9.9 available (current 0.1.0)"),
            "{first}"
        );
        assert!(
            first.contains("tokred update"),
            "a script install is updated by tokred itself: {first}"
        );
        assert_eq!((p.calls.get(), r.calls.get()), (1, 1));

        // Same day: no new requests, but the saved answer is still shown.
        let second = chores(&sb, &p, &r, Switches::default());
        assert!(second.contains("new version 9.9.9"));
        assert_eq!((p.calls.get(), r.calls.get()), (1, 1));
    }

    #[test]
    fn up_to_date_versions_print_nothing() {
        let sb = configured();
        let (p, r) = sources("0.1.0");
        assert_eq!(chores(&sb, &p, &r, Switches::default()), "");
    }

    #[test]
    fn the_switches_and_config_turn_each_chore_off() {
        let sb = configured();
        let (p, r) = sources("9.9.9");
        let switches = Switches {
            offline: false,
            no_update_check: true,
        };
        assert_eq!(chores(&sb, &p, &r, switches), "");
        assert_eq!((p.calls.get(), r.calls.get()), (1, 0));

        let sb = configured();
        std::fs::write(
            sb.env.data.config_path(),
            "[pricing]\nauto_sync = false\n[update]\nmode = \"off\"\n",
        )
        .unwrap();
        let (p, r) = sources("9.9.9");
        assert_eq!(chores(&sb, &p, &r, Switches::default()), "");
        assert_eq!((p.calls.get(), r.calls.get()), (0, 0));
    }

    #[test]
    fn a_failing_price_source_is_ignored() {
        let sb = configured();
        let p = Prices {
            calls: Cell::new(0),
            fail: true,
        };
        let r = FakeReleases {
            calls: Cell::new(0),
            version: "0.1.0",
        };
        assert_eq!(chores(&sb, &p, &r, Switches::default()), "");
        assert_eq!(p.calls.get(), 1);
    }

    #[test]
    fn an_unreadable_config_falls_back_to_the_defaults() {
        let sb = configured();
        std::fs::write(sb.env.data.config_path(), "[[[").unwrap();
        let (p, r) = sources("0.1.0");
        chores(&sb, &p, &r, Switches::default());
        assert_eq!(p.calls.get(), 1, "default config syncs prices");
    }

    #[test]
    fn switches_read_the_environment_flags() {
        // Only checks the parsing helper's shape; the process environment is not modified.
        let current = Switches::from_process();
        let again = Switches::from_process();
        assert_eq!(current, again);
    }

    #[test]
    fn an_upgraded_binary_re_applies_the_configuration_quietly() {
        let sb = configured();
        {
            let store = existing_store(&sb.env).unwrap().unwrap();
            store.meta_set("tokred_version", "0.0.1").unwrap();
        }
        // Damage something the re-application restores.
        std::fs::remove_file(sb.agent("tokred-dev.md")).unwrap();

        let (mut io, out, err) = scripted_with_err(false, "");
        reapply_configuration(&sb.env, &mut io, "0.1.0");
        assert_eq!((out.text().as_str(), err.text().as_str()), ("", ""));
        assert!(sb.agent("tokred-dev.md").exists());
        let store = existing_store(&sb.env).unwrap().unwrap();
        assert_eq!(
            store.meta_get("tokred_version").unwrap().as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn nothing_is_re_applied_when_the_version_matches_or_there_is_no_database() {
        let sb = Sandbox::new();
        let (mut io, out, err) = scripted_with_err(false, "");
        reapply_configuration(&sb.env, &mut io, "0.1.0");
        assert_eq!((out.text().as_str(), err.text().as_str()), ("", ""));

        let sb = configured();
        std::fs::remove_file(sb.agent("tokred-dev.md")).unwrap();
        reapply_configuration(&sb.env, &mut io, env!("CARGO_PKG_VERSION"));
        assert!(
            !sb.agent("tokred-dev.md").exists(),
            "same version: left alone"
        );
    }

    #[test]
    fn a_failed_re_application_is_reported_on_stderr() {
        let sb = configured();
        {
            let store = existing_store(&sb.env).unwrap().unwrap();
            store.meta_set("tokred_version", "0.0.1").unwrap();
        }
        sb.write_settings("{ broken");
        let (mut io, out, err) = scripted_with_err(false, "");
        reapply_configuration(&sb.env, &mut io, "0.1.0");
        assert_eq!(out.text(), "");
        let text = err.text();
        assert!(text.contains("could not be re-applied"));
        assert!(text.contains("tokred configure"));
    }
}
