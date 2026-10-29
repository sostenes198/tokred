//! The event bus of the dashboard: a bounded broadcast channel of server-sent-event frames.
//! Publishing never waits and costs nothing when nobody listens; a subscriber that falls behind
//! loses the oldest events (it re-reads the state from the API when it reconnects).

use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use serde::Serialize;
use tokio::sync::broadcast;

/// Events kept for a slow subscriber before it starts losing the oldest.
const CAPACITY: usize = 512;

/// A comment frame that keeps idle connections open.
pub(crate) const PING_FRAME: &[u8] = b": ping\n\n";

/// One server-sent event, already framed for the wire.
#[derive(Debug)]
pub(crate) struct Event {
    /// The session the event is about (for the `?session=` filter).
    pub session_id: String,
    /// `id:`, `event:` and `data:` lines and the blank line that ends the event.
    pub frame: Bytes,
}

/// The bus. One per proxy.
#[derive(Debug)]
pub(crate) struct EventBus {
    sender: broadcast::Sender<Arc<Event>>,
    last_id: Mutex<u64>,
}

impl EventBus {
    pub(crate) fn new() -> Self {
        Self {
            sender: broadcast::channel(CAPACITY).0,
            last_id: Mutex::new(0),
        }
    }

    /// Someone is listening; check before building an event.
    pub(crate) fn has_subscribers(&self) -> bool {
        self.sender.receiver_count() > 0
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.sender.subscribe()
    }

    /// Frames `data` (one JSON line) as event `name` and hands it to every subscriber. Does
    /// nothing when there are none.
    pub(crate) fn publish<T: Serialize>(&self, name: &str, session_id: &str, data: &T) {
        if !self.has_subscribers() {
            return;
        }
        let json = serde_json::to_string(data).unwrap_or_else(|_| "null".to_owned());
        // The id is taken and the event sent under one lock, so ids reach everybody in order.
        let mut last = self.last_id.lock().unwrap_or_else(PoisonError::into_inner);
        *last += 1;
        let frame = format!("id: {}\nevent: {name}\ndata: {json}\n\n", *last);
        let event = Arc::new(Event {
            session_id: session_id.to_owned(),
            frame: Bytes::from(frame),
        });
        // An error only means the last subscriber just left.
        let _gone = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};

    use super::*;

    fn text(event: &Event) -> String {
        String::from_utf8(event.frame.to_vec()).unwrap()
    }

    #[test]
    fn without_subscribers_nothing_is_built_or_counted() {
        let bus = EventBus::new();
        assert!(!bus.has_subscribers());
        bus.publish("session", "s", &json!({"a": 1}));
        let mut late = bus.subscribe();
        assert!(bus.has_subscribers());
        assert!(matches!(late.try_recv(), Err(TryRecvError::Empty)));
        bus.publish("session", "s", &json!({"a": 1}));
        assert!(
            text(&late.try_recv().unwrap()).starts_with("id: 1\n"),
            "the unseen event did not use an id"
        );
    }

    #[test]
    fn frames_carry_an_incrementing_id_an_event_name_and_one_data_line() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.publish("turn_start", "s1", &json!({"text": "two\nlines"}));
        bus.publish("turn_end", "s2", &json!(null));
        let first = rx.try_recv().unwrap();
        assert_eq!(first.session_id, "s1");
        assert_eq!(
            text(&first),
            "id: 1\nevent: turn_start\ndata: {\"text\":\"two\\nlines\"}\n\n"
        );
        assert_eq!(
            text(&rx.try_recv().unwrap()),
            "id: 2\nevent: turn_end\ndata: null\n\n"
        );
    }

    #[test]
    fn a_slow_subscriber_loses_the_oldest_events_and_publishing_never_blocks() {
        let bus = EventBus::new();
        let mut slow = bus.subscribe();
        for n in 0..CAPACITY + 10 {
            bus.publish("turn_delta", "s", &json!({"n": n}));
        }
        assert!(matches!(slow.try_recv(), Err(TryRecvError::Lagged(10))));
        assert!(text(&slow.try_recv().unwrap()).starts_with("id: 11\n"));
    }

    #[tokio::test]
    async fn a_closed_bus_ends_the_receiver() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        drop(bus);
        assert!(matches!(rx.recv().await, Err(RecvError::Closed)));
    }
}
