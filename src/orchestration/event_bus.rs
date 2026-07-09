//! Simple asynchronous event bus using `tokio::sync::broadcast`.
//!
//! Provides a global channel for publishing domain events (e.g. `NoteChanged`)
//! and allows multiple subscribers to process them concurrently.
//!
//! # Panics
//! Functions in this module will panic if called before `init_event_bus()` has
//! been called at least once, but the lazy-initialized `bus()` accessor ensures
//! this by constructing a default channel on first use.

use chrono::{DateTime, Utc};
use std::sync::OnceLock;
use tokio::sync::broadcast::{self, Receiver, Sender};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Channel capacity — must be large enough to absorb bursts when subscribers
// are temporarily slow.  Events that overflow the buffer are dropped.
// ---------------------------------------------------------------------------
const CHANNEL_CAPACITY: usize = 1024;

/// Global sender singleton.
fn sender() -> &'static Sender<ArcEvent> {
    static SENDER: OnceLock<Sender<ArcEvent>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        tx
    })
}

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// The action that triggered a `NoteChanged` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteAction {
    Created,
    Updated,
    Deleted,
}

/// Payload for a note change event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteChanged {
    pub note_id: String,
    /// A short text snippet (first ~500 chars) for lightweight analysis.
    pub content_snippet: String,
    pub timestamp: DateTime<Utc>,
    pub action: NoteAction,
}

/// Top-level event that can be published on the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    NoteChanged(NoteChanged),
}

// Because Event is not `Send` automatically via serde derives alone, wrap it.
type ArcEvent = std::sync::Arc<Event>;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Publish an event to all subscribers.
///
/// If all receivers are lagging and the channel is full the event is silently
/// dropped (broadcast behaviour).  Returns the number of active subscribers
/// that received the event.
pub fn publish(event: Event) -> usize {
    let payload = std::sync::Arc::new(event);
    sender().send(payload).unwrap_or(0)
}

/// Subscribe to all events.
///
/// The returned receiver will see every event published after this call.
/// If the receiver is slow and the channel buffer overflows, the *oldest*
/// events are dropped (lagged receivers get `RecvError::Lagged`).
pub fn subscribe() -> Receiver<ArcEvent> {
    sender().subscribe()
}

/// Convenience: publish a `NoteChanged` event in one call.
pub fn publish_note_changed(note_id: String, content_snippet: String, action: NoteAction) -> usize {
    publish(Event::NoteChanged(NoteChanged {
        note_id,
        content_snippet,
        timestamp: Utc::now(),
        action,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::TryRecvError;

    #[test]
    fn publish_and_receive_event() {
        let mut rx = subscribe();
        // Drain any pending messages from previous tests
        while rx.try_recv().is_ok() {}
        let n = publish_note_changed("test-123".into(), "hello world".into(), NoteAction::Created);
        // At least our own receiver got it
        assert!(n >= 1, "expected at least 1 subscriber, got {n}");

        let arc = rx.try_recv().expect("should have received event");
        match arc.as_ref() {
            Event::NoteChanged(ev) => {
                assert_eq!(ev.note_id, "test-123");
                assert_eq!(ev.action, NoteAction::Created);
                assert_eq!(ev.content_snippet, "hello world");
            }
        }
    }

    #[test]
    fn multiple_subscribers_all_get_event() {
        let mut rx1 = subscribe();
        let mut rx2 = subscribe();

        publish_note_changed("multi".into(), "data".into(), NoteAction::Updated);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn no_event_means_recv_error() {
        let mut rx = subscribe();
        // Flush any stale events that might be lingering
        loop {
            match rx.try_recv() {
                Ok(_) => continue,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(n)) => {
                    eprintln!("lagged by {n}, continuing");
                    continue;
                }
                Err(TryRecvError::Closed) => panic!("channel closed"),
            }
        }
        // Retry loop: another test may send events concurrently (#1705 race fix)
        for _ in 0..20 {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => return, // expected — success
                Ok(_) | Err(TryRecvError::Lagged(_)) => {
                    // Drain and retry
                    let _ = rx.try_recv();
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                Err(TryRecvError::Closed) => break,
            }
        }
        panic!("expected Empty after 20 retries — concurrent test is flooding the channel");
    }
}
