//! Choosing the proxy port: what a valid port is and whether it can be used.

use std::net::TcpListener;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::daemon::{ProxyFiles, healthy};
use crate::io::Io;

/// The port the proxy uses when nobody chose another one.
pub const DEFAULT_PORT: u16 = 1029;
/// Ports below this need elevated rights on Unix; they are never offered.
pub const MIN_PORT: u16 = 1024;

/// How long the "is it ours?" health request may take.
const OURS_TIMEOUT: Duration = Duration::from_millis(500);

/// What is listening on a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortState {
    /// Nothing: the proxy can bind it.
    Free,
    /// The proxy of this installation already listens there.
    Ours,
    /// Another program.
    Busy,
}

/// Parses a port typed by the user: a number from [`MIN_PORT`] to 65535.
pub fn parse_port(text: &str) -> Result<u16, String> {
    let text = text.trim();
    text.parse::<u16>()
        .ok()
        .filter(|port| *port >= MIN_PORT)
        .ok_or_else(|| format!("`{text}` is not a port number between {MIN_PORT} and 65535."))
}

/// `bind` (a `host:port` address) with its port replaced; a bind without a host keeps loopback.
pub fn with_port(bind: &str, port: u16) -> String {
    let host = bind.rsplit_once(':').map_or("127.0.0.1", |(host, _)| host);
    format!("{host}:{port}")
}

/// Tries to bind `127.0.0.1:<port>`; when that fails, asks the proxy of this installation (the
/// one in `files`) whether it is the one listening.
pub fn probe(files: &ProxyFiles, port: u16) -> PortState {
    if TcpListener::bind(("127.0.0.1", port)).is_ok() {
        return PortState::Free;
    }
    let recorded = files
        .read_pid()
        .is_some_and(|(_, recorded)| recorded == port);
    if recorded && healthy(port, OURS_TIMEOUT) {
        PortState::Ours
    } else {
        PortState::Busy
    }
}

/// Tells the user that a port given up front (a flag or `TOKRED_PORT`) is taken: it is still
/// used, but the proxy cannot start until it is free.
pub fn warn_if_busy(io: &mut Io, probe: &dyn Fn(u16) -> PortState, port: u16) -> Result<()> {
    if probe(port) == PortState::Busy {
        let warning = format!(
            "Port {port} is in use by another program: the proxy cannot start until it is free (or choose another with `tokred proxy enable --port <port>`).\n"
        );
        write!(io.out(), "{warning}")?;
    }
    Ok(())
}

/// Asks for the proxy port until it is valid and free (or already ours).
pub fn ask(io: &mut Io, probe: &dyn Fn(u16) -> PortState, default: u16) -> Result<u16> {
    loop {
        let answer = io.ask(&format!("Port for the tokred proxy [{default}]: "))?;
        let at_end = answer.is_none();
        let typed = answer.unwrap_or_default();
        let parsed = if typed.is_empty() {
            Ok(default)
        } else {
            parse_port(&typed)
        };
        match parsed {
            Err(message) => writeln!(io.out(), "{message}")?,
            Ok(port) if probe(port) == PortState::Busy => {
                let message = format!("Port {port} is in use by another program. Choose another.");
                writeln!(io.out(), "{message}")?;
                if at_end {
                    bail!("no free port was given for the proxy");
                }
            }
            Ok(port) => return Ok(port),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    fn files() -> (tempfile::TempDir, ProxyFiles) {
        let dir = tempfile::tempdir().unwrap();
        let files = ProxyFiles::new(dir.path());
        (dir, files)
    }

    #[test]
    fn ports_must_be_numbers_between_1024_and_65535() {
        assert_eq!(parse_port("1029"), Ok(1029));
        assert_eq!(parse_port(" 1024 \n"), Ok(1024));
        assert_eq!(parse_port("65535"), Ok(65535));
        for bad in ["", "abc", "80", "1023", "65536", "-5", "10.5"] {
            let message = parse_port(bad).unwrap_err();
            assert!(
                message.contains("between 1024 and 65535"),
                "{bad}: {message}"
            );
        }
    }

    #[test]
    fn replacing_the_port_keeps_the_host() {
        assert_eq!(with_port("127.0.0.1:1029", 2000), "127.0.0.1:2000");
        assert_eq!(with_port("0.0.0.0:1029", 2000), "0.0.0.0:2000");
        assert_eq!(with_port("nonsense", 2000), "127.0.0.1:2000");
    }

    #[test]
    fn a_free_port_is_free_and_a_held_one_is_busy() {
        let (_dir, files) = files();
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        assert_eq!(probe(&files, port), PortState::Busy);
        drop(held);
        assert_eq!(probe(&files, port), PortState::Free);
    }

    #[test]
    fn the_proxy_of_this_installation_is_recognised_by_its_pid_file_and_health_answer() {
        let (_dir, files) = files();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = r#"{"ok":true}"#;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        // Listening and healthy, but not recorded as ours: somebody else's.
        assert_eq!(probe(&files, port), PortState::Busy);
        files.write_pid(4242, port).unwrap();
        assert_eq!(probe(&files, port), PortState::Ours);
    }
    fn busy_on_3000(port: u16) -> PortState {
        if port == 3000 {
            PortState::Busy
        } else {
            PortState::Free
        }
    }

    #[test]
    fn the_port_is_asked_until_it_is_valid_and_free() {
        let (mut io, out) = crate::io::testing::scripted(true, "abc\n80\n3000\n3001\n");
        let port = ask(&mut io, &busy_on_3000, 1029).unwrap();
        assert_eq!(port, 3001);
        let text = out.text();
        assert!(
            text.contains("Port for the tokred proxy [1029]: "),
            "{text}"
        );
        assert!(text.contains("`abc` is not a port number"), "{text}");
        assert!(text.contains("`80` is not a port number"), "{text}");
        assert!(
            text.contains("Port 3000 is in use by another program. Choose another."),
            "{text}"
        );
    }

    #[test]
    fn enter_takes_the_default_and_a_port_that_is_ours_is_accepted() {
        let (mut io, _) = crate::io::testing::scripted(true, "\n");
        assert_eq!(ask(&mut io, &|_| PortState::Free, 2222).unwrap(), 2222);
        let (mut io, _) = crate::io::testing::scripted(true, "4000\n");
        assert_eq!(ask(&mut io, &|_| PortState::Ours, 1029).unwrap(), 4000);
    }

    #[test]
    fn a_busy_default_at_the_end_of_input_gives_up_instead_of_asking_forever() {
        let (mut io, _) = crate::io::testing::scripted(true, "");
        let error = ask(&mut io, &|_| PortState::Busy, 1029).unwrap_err();
        assert!(
            error.to_string().contains("no free port was given"),
            "{error}"
        );
    }

    #[test]
    fn a_busy_port_given_up_front_only_warns() {
        let (mut io, out) = crate::io::testing::scripted(false, "");
        warn_if_busy(&mut io, &busy_on_3000, 3000).unwrap();
        let text = out.text();
        assert!(
            text.contains("Port 3000 is in use by another program"),
            "{text}"
        );
        assert!(text.contains("tokred proxy enable --port"), "{text}");
        let (mut io, out) = crate::io::testing::scripted(false, "");
        warn_if_busy(&mut io, &busy_on_3000, 3001).unwrap();
        assert_eq!(out.text(), "");
    }
}
