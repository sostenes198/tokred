//! `tokred metrics`: the terminal dashboard plus the HTML report.

use std::path::PathBuf;

use anyhow::Result;
use tokred_audit::report::AuditReport;
use tokred_core::meta::MetaRepo;
use tokred_install::StdFileSystem;
use tokred_metrics::Stats;
use tokred_metrics::latency_stats;
use tokred_pricing::PRICE_SYNCED_AT;
use tokred_report::{Opener, render_html, write_report};

use super::data::{Loaded, load, load_from};
use super::report_data::{build, model_stats};
use super::{audit, show};
use crate::env::Env;
use crate::io::Io;
use crate::term::Term;
use crate::view::render_stats;

pub fn run(
    env: &Env,
    io: &mut Io,
    dir_flag: Option<PathBuf>,
    since: &str,
    no_open: bool,
    opener: &dyn Opener,
) -> Result<()> {
    let loaded = load(env, io, dir_flag, since)?;
    let audit_report = audit::default_project()
        .ok()
        .map(|project| audit::build_report(env, &loaded.claude_dir, project));
    let (html, stats) = render(&loaded, audit_report.as_ref())?;
    let path = env.data.reports_dir().join("metrics.html");
    write_report(&StdFileSystem, &path, &html)?;

    let dashboard = render_stats(Term::new(io.color()), &stats, &loaded.label);
    write!(io.out(), "{dashboard}")?;
    writeln!(io.out(), "\nHTML report written to {}", show(&path))?;
    if !no_open && let Err(err) = opener.open(&path) {
        writeln!(io.err(), "could not open the report in a browser: {err}")?;
    }
    Ok(())
}

/// The HTML report and the per-model statistics it was built from.
fn render(loaded: &Loaded, audit_report: Option<&AuditReport>) -> Result<(String, Stats)> {
    let stats = model_stats(loaded, &loaded.prices);
    let latency = latency_stats(&loaded.events, loaded.spawn_overhead_us, loaded.range);
    let last_sync = loaded
        .store
        .meta_get(PRICE_SYNCED_AT)?
        .and_then(|v| v.parse::<i64>().ok());
    let html = render_html(&build(loaded, &stats, &latency, audit_report, last_sync));
    Ok((html, stats))
}

/// The HTML report of the last `since` for the Claude Code folder `claude_dir`, without the
/// project audit (the proxy has no project of its own), for the proxy's `/metrics`.
pub fn html_report(env: &Env, claude_dir: PathBuf, since: &str) -> Result<String> {
    let loaded = load_from(env, claude_dir, since)?;
    render(&loaded, None).map(|(html, _)| html)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;

    use tokred_report::ReportError;

    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted_with_err;

    struct Browser {
        opened: RefCell<Vec<String>>,
        fail: bool,
    }

    impl Opener for Browser {
        fn open(&self, path: &Path) -> Result<(), ReportError> {
            self.opened.borrow_mut().push(path.display().to_string());
            if self.fail {
                return Err(ReportError::Open {
                    path: path.display().to_string(),
                    message: "no browser".to_owned(),
                });
            }
            Ok(())
        }
    }

    fn go(sb: &Sandbox, no_open: bool, browser: &Browser) -> (Result<()>, String, String) {
        let (mut io, out, err) = scripted_with_err(false, "");
        let result = run(
            &sb.env,
            &mut io,
            Some(sb.claude.clone()),
            "30d",
            no_open,
            browser,
        );
        (result, out.text(), err.text())
    }

    fn seeded() -> Sandbox {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        sb.seed_events();
        sb
    }

    #[test]
    fn it_prints_the_dashboard_writes_the_report_and_opens_it() {
        let sb = seeded();
        let browser = Browser {
            opened: RefCell::default(),
            fail: false,
        };
        let (result, out, err) = go(&sb, false, &browser);
        result.unwrap();

        assert!(out.contains("tokred Token Savings (last 30 days)"), "{out}");
        let path = sb.env.data.reports_dir().join("metrics.html");
        assert!(out.contains(&format!("HTML report written to {}", path.display())));
        let html = std::fs::read_to_string(&path).unwrap();
        assert!(
            html.contains("<!DOCTYPE html") || html.contains("<html"),
            "not an HTML document"
        );
        assert!(html.contains("claude-opus-4-1-20250805"));
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "the report must be offline"
        );
        assert_eq!(
            browser.opened.borrow().as_slice(),
            [path.display().to_string()]
        );
        assert_eq!(err, "");
    }

    #[test]
    fn the_same_file_is_overwritten_and_no_open_skips_the_browser() {
        let sb = seeded();
        let browser = Browser {
            opened: RefCell::default(),
            fail: false,
        };
        go(&sb, true, &browser).0.unwrap();
        go(&sb, true, &browser).0.unwrap();
        assert!(browser.opened.borrow().is_empty());
        let files: Vec<_> = std::fs::read_dir(sb.env.data.reports_dir())
            .unwrap()
            .collect();
        assert_eq!(files.len(), 1, "always the same file");
    }

    #[test]
    fn a_missing_browser_is_a_note_not_a_failure() {
        let sb = seeded();
        let browser = Browser {
            opened: RefCell::default(),
            fail: true,
        };
        let (result, _, err) = go(&sb, false, &browser);
        result.unwrap();
        assert!(
            err.contains("could not open the report in a browser"),
            "{err}"
        );
    }

    #[test]
    fn an_empty_database_still_produces_a_report() {
        let sb = Sandbox::new();
        sb.configure();
        let browser = Browser {
            opened: RefCell::default(),
            fail: false,
        };
        let (result, out, _) = go(&sb, true, &browser);
        result.unwrap();
        assert!(out.contains("No Claude Code usage found"));
        let html = std::fs::read_to_string(sb.env.data.reports_dir().join("metrics.html")).unwrap();
        assert!(
            html.contains("No data yet"),
            "empty sections are shown, not dropped"
        );
    }
}
