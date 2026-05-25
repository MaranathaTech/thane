//! Mock sink that fails N times then succeeds; verifies the dispatcher's
//! retry-with-backoff path.
//!
//! We shorten the test by using a sink that returns Transient errors twice
//! and then succeeds, and we wait in real time (the dispatcher's backoff is
//! 1s + 2s = 3s + batch interval, so ~10s tops).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use thane_audit_sink::{
    AuditDispatcher, AuditSink, DispatcherConfig, DeadLetterQueue, SinkError,
};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use uuid::Uuid;

struct FlakySink {
    attempts: Arc<AtomicU32>,
    fail_first: u32,
    delivered: Arc<AtomicU32>,
}

#[async_trait]
impl AuditSink for FlakySink {
    fn name(&self) -> &str { "flaky" }
    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if n <= self.fail_first {
            return Err(SinkError::Transient(format!("synthetic fail #{n}")));
        }
        self.delivered.fetch_add(batch.len() as u32, Ordering::SeqCst);
        Ok(())
    }
}

fn fake_event() -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        workspace_id: Uuid::new_v4(),
        panel_id: None,
        event_type: AuditEventType::CommandExecuted,
        severity: AuditSeverity::Info,
        description: "x".to_string(),
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
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_then_success_retries_with_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let attempts = Arc::new(AtomicU32::new(0));
    let delivered = Arc::new(AtomicU32::new(0));
    let sink = Arc::new(FlakySink {
        attempts: attempts.clone(),
        fail_first: 2,
        delivered: delivered.clone(),
    });
    let dlq = Arc::new(DeadLetterQueue::new(dir.path()));
    let dispatcher = AuditDispatcher::spawn(DispatcherConfig {
        sinks: vec![sink],
        dlq: dlq.clone(),
    });
    let handle = dispatcher.handle();

    handle.try_send(&fake_event());

    // Worst case: batch interval (5s) + 1s + 2s backoffs = 8s. Allow 15s.
    wait_until(Duration::from_secs(15), || delivered.load(Ordering::SeqCst) >= 1).await;

    assert_eq!(attempts.load(Ordering::SeqCst), 3, "should have tried 3 times (2 fails + 1 success)");
    assert_eq!(delivered.load(Ordering::SeqCst), 1, "exactly 1 event ultimately delivered");
    assert_eq!(dlq.count(), 0, "no DLQ entries on eventual success");
}
