//! `tokred dashboard`: opens the proxy's live dashboard in the browser. It starts nothing: the
//! proxy must be running. The proxy trades the admin secret for a one-time token that the
//! browser redeems for a session cookie, so the URL printed here works once and expires.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokred_core::Config;
use tokred_report::Opener;

use super::proxy::{admin, files};
use crate::daemon::{base_url, healthy, port_of};
use crate::env::Env;
use crate::io::Io;

/// Seconds the token in the printed URL stays valid (what the proxy reports; shown to the user).
const FALLBACK_TTL_SECS: u64 = 60;

/// The URL that signs a browser in with `token`.
fn sign_in_url(port: u16, token: &str) -> String {
    format!("{}/dashboard?token={token}", base_url(port))
}

/// Asks the running proxy for a token and opens (or prints) the dashboard URL.
pub fn run(
    env: &Env,
    io: &mut Io,
    config: &Config,
    no_open: bool,
    opener: &dyn Opener,
) -> Result<()> {
    let files = files(env);
    let port = files
        .read_pid()
        .map_or_else(|| port_of(&config.proxy.bind), |(_, port)| Ok(port))?;
    if !healthy(port, super::proxy::ADMIN_TIMEOUT) {
        bail!(
            "the tokred proxy is not running on {}; start it with `tokred proxy start` (or `tokred claude`)",
            base_url(port)
        );
    }
    let answer = admin(
        port,
        &files.secret()?,
        "POST",
        tokred_proxy::DASHBOARD_TOKEN_PATH,
    )
    .context("could not get a sign-in token from the proxy (is it an older version?)")?;
    let answer: Value =
        serde_json::from_str(&answer).context("the proxy answered with something unexpected")?;
    let token = answer["token"]
        .as_str()
        .context("the proxy did not return a token")?;
    let seconds = answer["expires_in"].as_u64().unwrap_or(FALLBACK_TTL_SECS);
    let url = sign_in_url(port, token);

    if no_open {
        let text = format!(
            "Open this address in your browser to see the dashboard:\n{url}\nIt works once and expires in {seconds} seconds; run `tokred dashboard` again for a new one.\n"
        );
        write!(io.out(), "{text}")?;
        return Ok(());
    }
    let text = format!(
        "Opening the tokred dashboard in your browser.\nIf it does not open, use this address (it works once and expires in {seconds} seconds):\n{url}\n"
    );
    write!(io.out(), "{text}")?;
    if let Err(error) = opener.open(Path::new(&url)) {
        writeln!(io.err(), "could not open the browser: {error}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::time::Duration;

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

    /// A fake proxy: health, and the token endpoint (checking the secret).
    fn fake_proxy(secret: String, token_reply: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
                let authorized = request.contains(&format!("x-tokred-secret: {secret}"));
                let (code, body) = if request.starts_with("get /__tokred/health") {
                    (200, r#"{"ok":true}"#)
                } else if request.starts_with("post /__tokred/dashboard-token") && authorized {
                    (200, token_reply)
                } else {
                    (401, "{}")
                };
                let response = format!(
                    "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        port
    }

    fn config_on(port: u16) -> Config {
        let mut config = Config::default();
        config.proxy.bind = format!("127.0.0.1:{port}");
        config
    }

    fn go(
        sb: &Sandbox,
        config: &Config,
        no_open: bool,
        browser: &Browser,
    ) -> (Result<()>, String, String) {
        let (mut io, out, err) = scripted_with_err(false, "");
        let result = run(&sb.env, &mut io, config, no_open, browser);
        (result, out.text(), err.text())
    }

    fn browser(fail: bool) -> Browser {
        Browser {
            opened: RefCell::default(),
            fail,
        }
    }

    #[test]
    fn it_opens_the_sign_in_address_and_explains_that_it_works_once() {
        let sb = Sandbox::new();
        let secret = files(&sb.env).secret().unwrap();
        let port = fake_proxy(secret, r#"{"ok":true,"token":"abc123","expires_in":60}"#);
        let browser = browser(false);
        let (result, out, err) = go(&sb, &config_on(port), false, &browser);
        result.unwrap();
        let url = format!("http://127.0.0.1:{port}/dashboard?token=abc123");
        assert_eq!(
            browser.opened.borrow().as_slice(),
            std::slice::from_ref(&url)
        );
        assert!(
            out.contains(&url) && out.contains("works once") && out.contains("60 seconds"),
            "{out}"
        );
        assert_eq!(err, "");
    }

    #[test]
    fn no_open_only_prints_the_address() {
        let sb = Sandbox::new();
        let secret = files(&sb.env).secret().unwrap();
        let port = fake_proxy(secret, r#"{"token":"t0k","expires_in":30}"#);
        let browser = browser(false);
        let (result, out, _) = go(&sb, &config_on(port), true, &browser);
        result.unwrap();
        assert!(browser.opened.borrow().is_empty());
        assert!(
            out.contains(&format!("http://127.0.0.1:{port}/dashboard?token=t0k")),
            "{out}"
        );
        assert!(out.contains("expires in 30 seconds"), "{out}");
    }

    #[test]
    fn a_missing_browser_is_a_note_not_a_failure() {
        let sb = Sandbox::new();
        let secret = files(&sb.env).secret().unwrap();
        let port = fake_proxy(secret, r#"{"token":"x"}"#);
        let browser = browser(true);
        let (result, out, err) = go(&sb, &config_on(port), false, &browser);
        result.unwrap();
        assert!(err.contains("could not open the browser"), "{err}");
        assert!(out.contains("expires in 60 seconds"), "{out}");
    }

    #[test]
    fn a_proxy_that_is_not_running_is_reported() {
        let sb = Sandbox::new();
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let browser = browser(false);
        let (result, out, _) = go(&sb, &config_on(closed), false, &browser);
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("not running") && message.contains("tokred proxy start"),
            "{message}"
        );
        assert_eq!(out, "");
        assert!(browser.opened.borrow().is_empty());
    }

    #[test]
    fn a_proxy_without_the_endpoint_or_with_a_bad_answer_is_reported() {
        let sb = Sandbox::new();
        // The fake only knows the token endpoint with the right secret.
        let port = fake_proxy("some other secret".to_owned(), r#"{"token":"x"}"#);
        let (result, _, _) = go(&sb, &config_on(port), true, &browser(false));
        assert!(result.unwrap_err().to_string().contains("sign-in token"));

        let secret = files(&sb.env).secret().unwrap();
        let port = fake_proxy(secret.clone(), "not json");
        let (result, _, _) = go(&sb, &config_on(port), true, &browser(false));
        assert!(result.unwrap_err().to_string().contains("unexpected"));

        let port = fake_proxy(secret, r#"{"ok":true}"#);
        let (result, _, _) = go(&sb, &config_on(port), true, &browser(false));
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("did not return a token")
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    #[test]
    fn the_address_carries_the_port_and_the_token() {
        assert_eq!(
            sign_in_url(1029, "t"),
            "http://127.0.0.1:1029/dashboard?token=t"
        );
    }
}
