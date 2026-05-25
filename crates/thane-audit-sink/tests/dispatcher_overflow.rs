//! When the dispatcher channel is full, push from the calling thread spills
//! to DLQ with reason `queue_overflow` rather than blocking.

use std::sync::Arc;

use async_trait::async_trait;
use thane_audit_sink::dispatcher::CHANNEL_CAPACITY;
use thane_audit_sink::{
    AuditDispatcher, AuditSink, DispatcherConfig, DeadLetterQueue, SinkError,
};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use tokio::sync::Notify;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

/// Sink that blocks until told to release. While it blocks, the dispatcher
/// channel fills up and incoming pushes have to spill to DLQ.
struct BlockingSink {
    release: Arc<Notify>,
}

#[async_trait]
impl AuditSink for BlockingSink {
    fn name(&self) -> &str { "blocked" }
    async fn send(&self, _batch: &[AuditEvent]) -> Result<(), SinkError> {
        self.release.notified().await;
        Ok(())
    }
}

fn ev() -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        workspace_id: Uuid::new_v4(),
        panel_id: None,
        event_type: AuditEventType::CommandExecuted,
        severity: AuditSeverity::Info,
        description: "x".into(),
        metadata: serde_json::json!({}),
        agent_name: None,
        system_user: None,
        system_uid: None,
        prev_hash: String::new(),
        hmac: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_channel_spills_to_dlq_with_overflow_reason() {
    let dir = tempfile::tempdir().unwrap();
    let release = Arc::new(Notify::new());
    let sink = Arc::new(BlockingSink { release: release.clone() });
    let dlq = Arc::new(DeadLetterQueue::new(dir.path()));
    let dispatcher = AuditDispatcher::spawn(DispatcherConfig {
        sinks: vec![sink],
        dlq: dlq.clone(),
    });
    let handle = dispatcher.handle();

    // Try to push 2x the channel capacity. Roughly half land in the channel,
    // the rest spill straight to DLQ.
    let target = CHANNEL_CAPACITY * 2;
    for _ in 0..target {
        handle.try_send(&ev());
    }

    // Give the dispatcher a chance to drain into the (blocked) sink.
    sleep(Duration::from_millis(200)).await;

    let dlq_count = dlq.count();
    assert!(
        dlq_count > 0,
        "spilling {target} events with capacity {CHANNEL_CAPACITY} should DLQ at least some"
    );
    let entries = dlq.read_all().unwrap();
    assert!(
        entries.iter().any(|e| e.error.contains("queue_overflow") || e.sink == "dispatcher"),
        "at least one entry should be marked as an overflow"
    );

    // Unblock the sink so the dispatcher can shut down cleanly when handle
    // is dropped.
    release.notify_waiters();
    drop(handle);
}
