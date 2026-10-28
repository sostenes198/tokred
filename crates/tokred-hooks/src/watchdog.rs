//! The time guard: a thread that gives up on a hook that takes too long (a blocked stdin, a
//! very slow disk) so that Claude Code is never kept waiting by tokred.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Cancels the guard when dropped.
#[derive(Debug)]
pub struct Watchdog {
    cancelled: Arc<AtomicBool>,
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// Starts a thread that runs `on_timeout` after `limit` unless the returned [`Watchdog`] was
/// dropped first. `on_timeout` is expected to record the run and exit the process with code 0.
///
/// Returns `None` when the thread cannot be spawned (the hook then simply has no guard).
pub fn arm(limit: Duration, on_timeout: impl FnOnce() + Send + 'static) -> Option<Watchdog> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    std::thread::Builder::new()
        .name("tokred-guard".to_owned())
        .spawn(move || {
            std::thread::sleep(limit);
            if !flag.load(Ordering::SeqCst) {
                on_timeout();
            }
        })
        .ok()?;
    Some(Watchdog { cancelled })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(flag: &Arc<AtomicBool>) -> impl FnOnce() + Send + 'static + use<> {
        let flag = Arc::clone(flag);
        move || flag.store(true, Ordering::SeqCst)
    }

    #[test]
    fn the_guard_fires_after_the_limit() {
        let fired = Arc::new(AtomicBool::new(false));
        let _guard = arm(Duration::from_millis(10), set(&fired));
        let mut waited = 0;
        while !fired.load(Ordering::SeqCst) && waited < 500 {
            std::thread::sleep(Duration::from_millis(10));
            waited += 1;
        }
        assert!(fired.load(Ordering::SeqCst), "the guard never fired");
    }

    #[test]
    fn a_dropped_guard_never_fires() {
        let fired = Arc::new(AtomicBool::new(false));
        drop(arm(Duration::from_millis(30), set(&fired)));
        std::thread::sleep(Duration::from_millis(300));
        assert!(!fired.load(Ordering::SeqCst));
    }
}
