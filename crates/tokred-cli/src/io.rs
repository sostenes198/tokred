//! Terminal I/O behind a small seam, so prompts and output can be tested without a TTY.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::Result;

/// Where a command reads answers from and writes its output to.
pub struct Io {
    interactive: bool,
    input: Box<dyn BufRead>,
    out: Box<dyn Write>,
}

impl Io {
    /// The real terminal: stdin, stdout, and whether stdin is a TTY.
    pub fn stdio() -> Self {
        Self::new(
            std::io::stdin().is_terminal(),
            Box::new(std::io::stdin().lock()),
            Box::new(std::io::stdout()),
        )
    }

    /// Builds an `Io` from explicit parts.
    pub fn new(interactive: bool, input: Box<dyn BufRead>, out: Box<dyn Write>) -> Self {
        Self {
            interactive,
            input,
            out,
        }
    }

    /// Whether a person can answer prompts.
    pub fn interactive(&self) -> bool {
        self.interactive
    }

    /// The output stream.
    pub fn out(&mut self) -> &mut dyn Write {
        &mut *self.out
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

    /// An `Io` that answers from `input` and records its output.
    pub fn scripted(interactive: bool, input: &str) -> (Io, Capture) {
        let capture = Capture::default();
        let io = Io::new(
            interactive,
            Box::new(std::io::Cursor::new(input.to_owned().into_bytes())),
            Box::new(capture.clone()),
        );
        (io, capture)
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
    fn out_writes_to_the_captured_stream() {
        let (mut io, out) = scripted(false, "");
        assert!(!io.interactive());
        std::io::Write::write_all(io.out(), b"hello").unwrap();
        assert_eq!(out.text(), "hello");
    }

    #[test]
    fn stdio_builds_without_reading_anything() {
        // Under `cargo test` stdin is not a terminal; just make sure construction works.
        let _io = super::Io::stdio();
    }
}
