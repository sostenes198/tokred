//! Terminal I/O behind a small seam, so prompts and output can be tested without a TTY.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::Result;

/// Where a command reads answers from and writes its output to.
pub struct Io {
    interactive: bool,
    color: bool,
    input: Box<dyn BufRead>,
    out: Box<dyn Write>,
    err: Box<dyn Write>,
}

impl Io {
    /// The real terminal: stdin, stdout, and whether stdin is a TTY.
    pub fn stdio() -> Self {
        let io = Self::new(
            std::io::stdin().is_terminal(),
            Box::new(std::io::stdin().lock()),
            Box::new(std::io::stdout()),
            Box::new(std::io::stderr()),
        );
        io.with_color(wants_color(std::io::stdout().is_terminal(), |name| {
            std::env::var_os(name)
        }))
    }

    /// Builds an `Io` from explicit parts.
    pub fn new(
        interactive: bool,
        input: Box<dyn BufRead>,
        out: Box<dyn Write>,
        err: Box<dyn Write>,
    ) -> Self {
        Self {
            interactive,
            color: false,
            input,
            out,
            err,
        }
    }

    /// Turns ANSI colors on or off for this stream.
    #[must_use]
    pub fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Whether output should be colored.
    pub fn color(&self) -> bool {
        self.color
    }

    /// Whether a person can answer prompts.
    pub fn interactive(&self) -> bool {
        self.interactive
    }

    /// The input and output streams together, for commands that read and write at once.
    /// Commands must read stdin through this (or `ask`), never by locking stdin again: the
    /// real `Io` already holds the lock for the whole process.
    pub fn streams(&mut self) -> (&mut dyn BufRead, &mut dyn Write) {
        (&mut *self.input, &mut *self.out)
    }

    /// The output stream.
    pub fn out(&mut self) -> &mut dyn Write {
        &mut *self.out
    }

    /// The error stream (notices that must not pollute `--json` output).
    pub fn err(&mut self) -> &mut dyn Write {
        &mut *self.err
    }

    /// Asks a question and returns the trimmed answer (`None` at end of input).
    pub fn ask(&mut self, question: &str) -> Result<Option<String>> {
        write!(self.out, "{question}")?;
        self.out.flush()?;
        let mut line = String::new();
        if self.input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim().to_owned()))
    }
}

/// Color policy: `NO_COLOR` always wins, `CLICOLOR_FORCE` forces color on, a dumb terminal
/// gets none, and otherwise color is used when stdout is a terminal.
fn wants_color(stdout_is_terminal: bool, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    if var("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return false;
    }
    if var("CLICOLOR_FORCE").is_some_and(|value| !value.is_empty() && value != "0") {
        return true;
    }
    stdout_is_terminal && var("TERM").is_none_or(|term| term != "dumb")
}

#[cfg(test)]
pub mod testing {
    //! Helpers for unit tests of the commands.

    use std::cell::RefCell;
    use std::io::Write;
    use std::rc::Rc;

    use super::Io;

    /// A cloneable in-memory output buffer.
    #[derive(Clone, Default)]
    pub struct Capture(Rc<RefCell<Vec<u8>>>);

    impl Capture {
        /// Everything written so far.
        pub fn text(&self) -> String {
            String::from_utf8(self.0.borrow().clone()).unwrap()
        }
    }

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// An `Io` that answers from `input` and records its output (stderr is discarded).
    pub fn scripted(interactive: bool, input: &str) -> (Io, Capture) {
        let (io, out, _err) = scripted_with_err(interactive, input);
        (io, out)
    }

    /// Like [`scripted`], also recording what is written to stderr.
    pub fn scripted_with_err(interactive: bool, input: &str) -> (Io, Capture, Capture) {
        let out = Capture::default();
        let err = Capture::default();
        let io = Io::new(
            interactive,
            Box::new(std::io::Cursor::new(input.to_owned().into_bytes())),
            Box::new(out.clone()),
            Box::new(err.clone()),
        );
        (io, out, err)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::scripted;

    #[test]
    fn ask_returns_trimmed_answers_then_none() {
        let (mut io, out) = scripted(true, "  yes \n");
        assert!(io.interactive());
        assert_eq!(io.ask("Sure? ").unwrap().as_deref(), Some("yes"));
        assert_eq!(io.ask("Again? ").unwrap(), None);
        assert_eq!(out.text(), "Sure? Again? ");
    }

    #[test]
    fn streams_read_and_write_without_a_second_stdin_lock() {
        let (mut io, out) = scripted(false, "payload\n");
        let (input, output) = io.streams();
        let mut line = String::new();
        input.read_line(&mut line).unwrap();
        write!(output, "got {}", line.trim()).unwrap();
        assert_eq!(out.text(), "got payload");
    }

    #[test]
    fn err_writes_to_its_own_stream() {
        let (mut io, out, err) = super::testing::scripted_with_err(false, "");
        std::io::Write::write_all(io.err(), b"notice").unwrap();
        assert_eq!(err.text(), "notice");
        assert_eq!(out.text(), "");
    }

    #[test]
    fn out_writes_to_the_captured_stream() {
        let (mut io, out) = scripted(false, "");
        assert!(!io.interactive());
        std::io::Write::write_all(io.out(), b"hello").unwrap();
        assert_eq!(out.text(), "hello");
    }

    #[test]
    fn the_color_policy_follows_no_color_clicolor_force_and_the_terminal() {
        use std::ffi::OsString;
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, v)| OsString::from(*v))
            }
        };
        assert!(super::wants_color(true, env(&[])));
        assert!(!super::wants_color(false, env(&[])));
        assert!(!super::wants_color(true, env(&[("NO_COLOR", "1")])));
        assert!(super::wants_color(true, env(&[("NO_COLOR", "")])));
        assert!(!super::wants_color(true, env(&[("TERM", "dumb")])));
        assert!(super::wants_color(true, env(&[("TERM", "xterm-256color")])));
        assert!(super::wants_color(false, env(&[("CLICOLOR_FORCE", "1")])));
        assert!(!super::wants_color(false, env(&[("CLICOLOR_FORCE", "0")])));
        assert!(!super::wants_color(
            true,
            env(&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")])
        ));
    }

    #[test]
    fn with_color_toggles_the_flag() {
        let (io, _) = scripted(false, "");
        assert!(!io.color());
        assert!(io.with_color(true).color());
    }

    #[test]
    fn stdio_builds_without_reading_anything() {
        // Under `cargo test` stdin is not a terminal; just make sure construction works.
        let _io = super::Io::stdio();
    }
}
