//! Events below the sink's `min_severity` must not reach `send`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use thane_audit_sink::{
    AuditDispatcher, AuditSink, DispatcherConfig, DeadLetterQueue, SinkError,
};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use uuid::Uuid;

struct CountingSink {
    seen: Arc<AtomicU32>,
    min: AuditSeverity,
}

#[async_trait]
impl AuditSink for CountingSink {
    fn name(&self) -> &str { "counting" }
    fn min_severity(&self) -> AuditSeverity { self.min }
    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        self.seen.fetch_add(batch.len() as u32, Ordering::SeqCst);
        Ok(())
    }
}

fn ev(sev: AuditSeverity) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        workspace_id: Uuid::new_v4(),
        panel_id: None,
        event_type: AuditEventType::CommandExecuted,
        severity: sev,
        description: "x".into(),
        metadata: serde_json::json!({}),
        agent_name: None,
        system_user: None,
        system_uid: None,
        prev_hash: String::new(),
        hmac: None,
    }
}

async fn wait_until<F: Fn() -> bool>(timeout: Duration, predicate: F) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if predicate() { return; }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn min_severity_drops_low_events() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(AtomicU32::new(0));
    let dlq = Arc::new(DeadLetterQueue::new(dir.path()));
    let dispatcher = AuditDispatcher::spawn(DispatcherConfig {
        sinks: vec![Arc::new(CountingSink { seen: seen.clone(), min: AuditSeverity::Alert })],
        dlq,
    });
    let handle = dispatcher.handle();

    handle.try_send(&ev(AuditSeverity::Info));
    handle.try_send(&ev(AuditSeverity::Warning));
    handle.try_send(&ev(AuditSeverity::Alert));
    handle.try_send(&ev(AuditSeverity::Critical));

    // Wait for the partial-batch interval to elapse + drain.
    wait_until(Duration::from_secs(7), || seen.load(Ordering::SeqCst) >= 2).await;

    assert_eq!(seen.load(Ordering::SeqCst), 2, "only Alert + Critical reach the sink");
}
