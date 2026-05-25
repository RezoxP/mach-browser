//! Single fan-out point for CDP events, MCP progress, and internal logging.
//!
//! Phase 0 only emits a minimal set of events (request start/end, parse
//! result). Later phases (CDP, MCP, JS) attach more producers and
//! consumers without changing this surface.

use std::sync::Arc;
use std::sync::Mutex;

/// Categorical tag for a [`Notification`]. Subscribers filter on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum NotificationKind {
    RequestStart,
    RequestComplete,
    RequestFailed,
    ParseComplete,
    ParseFailed,
}

/// A single event published on the bus.
#[derive(Debug, Clone)]
pub struct Notification {
    /// Category of the event.
    pub kind: NotificationKind,
    /// Free-form human-readable payload. Kept opaque for Phase 0; later
    /// phases will replace this with a structured per-kind payload enum.
    pub detail: String,
}

type Subscriber = Arc<dyn Fn(&Notification) + Send + Sync>;

/// Minimal pub/sub bus. Synchronous fan-out, in-process.
///
/// Use one instance per `App`. Cloning is cheap and shares state.
#[derive(Default, Clone)]
pub struct NotificationBus {
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
}

impl NotificationBus {
    /// Create an empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a subscriber. The closure receives every notification
    /// published to the bus; it should not block.
    pub fn subscribe<F>(&self, f: F)
    where
        F: Fn(&Notification) + Send + Sync + 'static,
    {
        // Lock poisoning here would mean a panic inside a subscriber.
        // We ignore poisoning and continue — losing a subscription is
        // strictly better than aborting the whole process.
        if let Ok(mut subs) = self.subscribers.lock() {
            subs.push(Arc::new(f));
        }
    }

    /// Publish an event. Returns silently if the lock is poisoned.
    pub fn publish(&self, n: Notification) {
        if let Ok(subs) = self.subscribers.lock() {
            for s in subs.iter() {
                s(&n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn fanout_to_multiple_subscribers() {
        let bus = NotificationBus::new();
        let count = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let c = count.clone();
            bus.subscribe(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }
        bus.publish(Notification {
            kind: NotificationKind::RequestStart,
            detail: "GET /".into(),
        });
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }
}
