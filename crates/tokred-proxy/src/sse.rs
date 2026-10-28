//! Incremental parser of a `text/event-stream` body (the WHATWG server-sent events format).
//! Chunks may split an event, a line, or a CRLF pair anywhere; the parser keeps the partial
//! line between calls. Observation only: the forwarded bytes never pass through it.

/// An event or line larger than this is dropped (the bytes are still forwarded).
const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

/// Parser state carried between chunks.
#[derive(Debug, Default)]
pub(crate) struct SseParser {
    line: Vec<u8>,
    pending_cr: bool,
    event: Option<String>,
    data: Vec<u8>,
    has_data: bool,
    overflow: bool,
}

/// Value of a field line: everything after the first colon, minus one leading space.
fn field_value(value: &[u8]) -> &[u8] {
    value.strip_prefix(b" ").unwrap_or(value)
}

impl SseParser {
    /// Feeds a chunk; `on_event(event_name, data)` runs for every complete event.
    pub(crate) fn feed(&mut self, chunk: &[u8], on_event: &mut dyn FnMut(Option<&str>, &[u8])) {
        for &byte in chunk {
            let after_cr = std::mem::take(&mut self.pending_cr);
            match byte {
                b'\n' if after_cr => {}
                b'\n' => self.end_line(on_event),
                b'\r' => {
                    self.pending_cr = true;
                    self.end_line(on_event);
                }
                _ if self.line.len() < MAX_EVENT_BYTES => self.line.push(byte),
                _ => self.overflow = true,
            }
        }
    }

    fn end_line(&mut self, on_event: &mut dyn FnMut(Option<&str>, &[u8])) {
        if self.line.is_empty() {
            self.dispatch(on_event);
            return;
        }
        let line = std::mem::take(&mut self.line);
        let (field, value) = match line.iter().position(|&b| b == b':') {
            Some(colon) => line.split_at(colon),
            None => (line.as_slice(), &b":"[..]),
        };
        let value = field_value(value.get(1..).unwrap_or_default());
        match field {
            b"event" => self.event = Some(String::from_utf8_lossy(value).into_owned()),
            b"data" if self.data.len() + value.len() < MAX_EVENT_BYTES => {
                if self.has_data {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value);
                self.has_data = true;
            }
            b"data" => self.overflow = true,
            // Comments (empty field name) and fields this tap does not use.
            _ => {}
        }
        self.line = line;
        self.line.clear();
    }

    fn dispatch(&mut self, on_event: &mut dyn FnMut(Option<&str>, &[u8])) {
        if self.has_data && !self.overflow {
            on_event(self.event.as_deref(), &self.data);
        }
        self.event = None;
        self.data.clear();
        self.has_data = false;
        self.overflow = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(chunks: &[&[u8]]) -> Vec<(Option<String>, String)> {
        let mut parser = SseParser::default();
        let mut out = Vec::new();
        for chunk in chunks {
            parser.feed(chunk, &mut |name, data| {
                out.push((
                    name.map(str::to_owned),
                    String::from_utf8_lossy(data).into_owned(),
                ));
            });
        }
        out
    }

    const STREAM: &[u8] = b"event: message_start\ndata: {\"a\":1}\n\n: comment\nevent: ping\r\ndata: {}\r\n\r\nevent: x\rdata:no-space\r\rdata: line1\ndata: line2\nid: 7\nretry: 5\n\n";

    fn expected() -> Vec<(Option<String>, String)> {
        vec![
            (Some("message_start".to_owned()), "{\"a\":1}".to_owned()),
            (Some("ping".to_owned()), "{}".to_owned()),
            (Some("x".to_owned()), "no-space".to_owned()),
            (None, "line1\nline2".to_owned()),
        ]
    }

    #[test]
    fn whole_stream_parses() {
        assert_eq!(events(&[STREAM]), expected());
    }

    #[test]
    fn every_split_point_gives_the_same_events() {
        for split in 0..=STREAM.len() {
            let (a, b) = STREAM.split_at(split);
            assert_eq!(events(&[a, b]), expected(), "split at {split}");
        }
        let bytes: Vec<&[u8]> = STREAM.chunks(1).collect();
        assert_eq!(events(&bytes), expected());
    }

    #[test]
    fn an_incomplete_event_is_not_dispatched() {
        assert!(events(&[b"event: message_stop\ndata: {}\n"]).is_empty());
        assert!(events(&[b"event: lonely\n\n"]).is_empty(), "no data");
        assert_eq!(
            events(&[b"data\n\n"]),
            vec![(None, String::new())],
            "a field without a colon has an empty value"
        );
    }

    #[test]
    fn oversized_lines_and_events_are_dropped_and_parsing_recovers() {
        let mut big = b"data: ".to_vec();
        big.resize(MAX_EVENT_BYTES + 10, b'x');
        big.extend_from_slice(b"\n\ndata: ok\n\n");
        assert_eq!(events(&[&big]), vec![(None, "ok".to_owned())]);

        let half = vec![b'y'; MAX_EVENT_BYTES / 2 + 1];
        let mut many = Vec::new();
        for _ in 0..3 {
            many.extend_from_slice(b"data: ");
            many.extend_from_slice(&half);
            many.push(b'\n');
        }
        many.extend_from_slice(b"\ndata: next\n\n");
        assert_eq!(events(&[&many]), vec![(None, "next".to_owned())]);
    }
}
