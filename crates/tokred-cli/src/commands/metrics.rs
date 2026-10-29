//! `tokred metrics`: the terminal dashboard plus the HTML report.

use std::path::PathBuf;

use anyhow::Result;
use tokred_install::StdFileSystem;
use tokred_report::{Opener, write_report};

use super::data::{Repos, load};
use super::report_data::{model_stats, report_html};
use super::{audit, show};
use crate::env::Env;
use crate::io::Io;
use crate::term::Term;
use crate::view::{render_proxy, render_stats};

pub fn run(
    env: &Env,
    io: &mut Io,
    dir_flag: Option<PathBuf>,
    since: &str,
    no_open: bool,
    opener: &dyn Opener,
) -> Result<()> {
    let loaded = load(env, io, dir_flag, since)?;
    let data = &loaded.data;
    let audit_report = audit::default_project()
        .ok()
        .map(|project| audit::build_report(env, &loaded.claude_dir, project));
    let html = report_html(
        &Repos::of(&loaded.store),
        data.prices.clone(),
        data.now,
        since,
        audit_report.as_ref(),
    )?;
    let path = env.data.reports_dir().join("metrics.html");
    write_report(&StdFileSystem, &path, &html)?;

    let term = Term::new(io.color());
    let mut dashboard = render_stats(term, &model_stats(data, &data.prices), &data.label);
    if let Some(proxy) = data.proxy() {
        dashboard.push_str(&render_proxy(term, &proxy));
    }
    write!(io.out(), "{dashboard}")?;
    writeln!(io.out(), "\nHTML report written to {}", show(&path))?;
    if !no_open && let Err(err) = opener.open(&path) {
        writeln!(io.err(), "could not open the report in a browser: {err}")?;
    }
    Ok(())
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
    #[test]
    fn the_report_and_the_dashboard_show_the_proxy_when_it_recorded_something() {
        let sb = seeded();
        sb.seed_proxy();
        let browser = Browser {
            opened: RefCell::default(),
            fail: false,
        };
        let (result, out, _) = go(&sb, true, &browser);
        result.unwrap();
        assert!(
            out.contains("Data source:  3 messages measured by the proxy"),
            "{out}"
        );
        let html = std::fs::read_to_string(sb.env.data.reports_dir().join("metrics.html")).unwrap();
        for id in [
            "proxy-routing",
            "proxy-overhead",
            "proxy-cache",
            "proxy-origin",
        ] {
            assert!(html.contains(id), "{id}");
        }
        assert!(html.contains("claude-haiku-4-5"));
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "still offline"
        );
    }

    #[test]
    fn without_the_proxy_the_report_has_no_proxy_sections() {
        let sb = seeded();
        let browser = Browser {
            opened: RefCell::default(),
            fail: false,
        };
        let (_, out, _) = go(&sb, true, &browser);
        assert!(!out.contains("Data source"), "{out}");
        let html = std::fs::read_to_string(sb.env.data.reports_dir().join("metrics.html")).unwrap();
        assert!(!html.contains("proxy-routing") && !html.contains("\"proxy\""));
    }
}
