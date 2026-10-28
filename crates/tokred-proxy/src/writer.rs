//! The single writer of proxy records (spec 4.2): requests queue records on a bounded channel
//! and one thread applies them (usually to SQLite). When the queue is full the record is
//! dropped and logged; a request never waits for storage.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use tokred_core::proxy::{ProxyRecord, RecordSink};

/// Records that can wait in the queue before new ones are dropped.
pub const DEFAULT_WRITER_CAPACITY: usize = 4096;

/// Applies one record to storage; an error is logged and the writer moves on.
pub type ApplyRecord = Box<dyn FnMut(&ProxyRecord) -> Result<(), String> + Send>;

/// What the writer thread did, returned when it ends (after every sink is dropped).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriterReport {
    /// Records applied successfully.
    pub written: u64,
    /// Records whose apply failed.
    pub failed: u64,
}

/// The sending side: a [`RecordSink`] that never blocks.
#[derive(Debug)]
pub struct ChannelSink {
    sender: SyncSender<ProxyRecord>,
    dropped: Arc<AtomicU64>,
}

impl ChannelSink {
    /// Records dropped because the queue was full or the writer had stopped.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl RecordSink for ChannelSink {
    fn record(&self, record: ProxyRecord) {
        if self.sender.try_send(record).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("proxy record dropped: the writer queue is full or stopped");
        }
    }
}

fn drain(receiver: &Receiver<ProxyRecord>, apply: &mut ApplyRecord) -> WriterReport {
    let mut report = WriterReport::default();
    for record in receiver {
        match apply(&record) {
            Ok(()) => report.written += 1,
            Err(error) => {
                report.failed += 1;
                tracing::warn!(%error, "could not store a proxy record");
            }
        }
    }
    report
}

/// Starts the writer thread with [`DEFAULT_WRITER_CAPACITY`].
#[must_use]
pub fn spawn_writer(apply: ApplyRecord) -> (ChannelSink, JoinHandle<WriterReport>) {
    spawn_writer_with_capacity(DEFAULT_WRITER_CAPACITY, apply)
}

/// Starts the writer thread with a queue of `capacity` records (at least one). The thread
/// ends, returning its report, once the sink is dropped and the queue is empty.
#[must_use]
pub fn spawn_writer_with_capacity(
    capacity: usize,
    mut apply: ApplyRecord,
) -> (ChannelSink, JoinHandle<WriterReport>) {
    let (sender, receiver) = sync_channel(capacity.max(1));
    let handle = std::thread::spawn(move || drain(&receiver, &mut apply));
    let sink = ChannelSink {
        sender,
        dropped: Arc::new(AtomicU64::new(0)),
    };
    (sink, handle)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc::channel;

    use tokred_core::proxy::ProxyRequestRecord;

    use super::*;

    fn request(id: &str) -> ProxyRecord {
        ProxyRecord::Request(Box::new(ProxyRequestRecord {
            id: id.to_owned(),
            ..ProxyRequestRecord::default()
        }))
    }

    #[test]
    fn records_are_applied_in_order_and_failures_counted() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let (sink, handle) = spawn_writer(Box::new(move |record| {
            let ProxyRecord::Request(request) = record else {
                return Err("unexpected".to_owned());
            };
            sink_seen.lock().unwrap().push(request.id.clone());
            if request.id == "bad" {
                Err("disk full".to_owned())
            } else {
                Ok(())
            }
        }));
        for id in ["a", "bad", "c"] {
            sink.record(request(id));
        }
        drop(sink);
        let report = handle.join().unwrap();
        assert_eq!(
            report,
            WriterReport {
                written: 2,
                failed: 1
            }
        );
        assert_eq!(*seen.lock().unwrap(), ["a", "bad", "c"]);
    }

    #[test]
    fn a_full_queue_drops_instead_of_blocking() {
        let (release, wait) = channel::<()>();
        let (sink, handle) = spawn_writer_with_capacity(
            0,
            Box::new(move |_| {
                wait.recv().map_err(|error| error.to_string())?;
                Ok(())
            }),
        );
        // The first record is taken by the thread (blocked in apply), the second fills the
        // queue of one, the rest are dropped.
        sink.record(request("1"));
        while sink.sender.try_send(request("probe")).is_ok() {
            std::thread::yield_now();
        }
        sink.record(request("dropped"));
        assert!(sink.dropped() >= 1);
        drop(release);
        drop(sink);
        let report = handle.join().unwrap();
        assert_eq!(
            report.written, 0,
            "every apply failed once the release closed"
        );
    }
}
