//! Thin handle wrapper around the dispatcher's MPSC sender.
//!
//! `AuditLog::record` owns one of these and uses `try_send` so a slow sink
//! never blocks the calling thread. Cloning the handle clones the channel
//! sender — every thread that records events can have its own.

use std::sync::Arc;
use tokio::sync::mpsc;

use thane_core::audit::{AuditEvent, AuditEventForwarder};

use crate::dlq::DeadLetterQueue;

/// Reason an event was dropped before reaching any sink.
pub const SPILL_REASON_OVERFLOW: &str = "queue_overflow";

/// Lightweight, clonable handle to a running dispatcher.
#[derive(Clone)]
pub struct DispatcherHandle {
    tx: mpsc::Sender<AuditEvent>,
    /// Shared DLQ used both by the dispatcher and by the handle (when it has
    /// to spill overflow events from the calling thread).
    dlq: Arc<DeadLetterQueue>,
}

impl DispatcherHandle {
    pub fn new(tx: mpsc::Sender<AuditEvent>, dlq: Arc<DeadLetterQueue>) -> Self {
        Self { tx, dlq }
    }

    /// Try to enqueue an event. Never blocks. If the channel is full, the
    /// event is written to the dead-letter queue immediately so it isn't lost.
    pub fn try_send(&self, event: &AuditEvent) {
        match self.tx.try_send(event.clone()) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(ev)) => {
                tracing::warn!(
                    "audit dispatcher queue full; spilling event {} to DLQ",
                    ev.id
                );
                self.dlq.write_overflow(&ev, SPILL_REASON_OVERFLOW);
            }
            Err(mpsc::error::TrySendError::Closed(ev)) => {
                tracing::warn!(
                    "audit dispatcher closed; spilling event {} to DLQ",
                    ev.id
                );
                self.dlq.write_overflow(&ev, "dispatcher_closed");
            }
        }
    }

    /// Number of events currently queued (best-effort; capacity - permits).
    pub fn queued(&self) -> usize {
        self.tx.max_capacity().saturating_sub(self.tx.capacity())
    }

    /// Total channel capacity.
    pub fn capacity(&self) -> usize {
        self.tx.max_capacity()
    }
}

impl AuditEventForwarder for DispatcherHandle {
    fn forward(&self, event: &AuditEvent) {
        self.try_send(event);
    }
}
