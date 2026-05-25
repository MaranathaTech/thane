//! A Permanent sink failure must DLQ on the first attempt — no retries.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use thane_audit_sink::{
    AuditDispatcher, AuditSink, DispatcherConfig, DeadLetterQueue, SinkError,
};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use uuid::Uuid;

struct AlwaysPermanent {
    attempts: Arc<AtomicU32>,
}

#[async_trait]
impl AuditSink for AlwaysPermanent {
    fn name(&self) -> &str { "perma" }
    async fn send(&self, _batch: &[AuditEvent]) -> Result<(), SinkError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(SinkError::Permanent("bad config".into()))
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
async fn permanent_failure_dlqs_immediately_no_retry() {
    let dir = tempfile::tempdir().unwrap();
    let attempts = Arc::new(AtomicU32::new(0));
    let sink = Arc::new(AlwaysPermanent { attempts: attempts.clone() });
    let dlq = Arc::new(DeadLetterQueue::new(dir.path()));
    let dispatcher = AuditDispatcher::spawn(DispatcherConfig {
        sinks: vec![sink],
        dlq: dlq.clone(),
    });
    let handle = dispatcher.handle();
    handle.try_send(&fake_event());

    // Batch interval is 5s. Allow 10s for the single Permanent attempt.
    wait_until(Duration::from_secs(10), || dlq.count() >= 1).await;
    // Give a little extra time to confirm no retries happen.
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(attempts.load(Ordering::SeqCst), 1, "Permanent should skip retries");
    assert_eq!(dlq.count(), 1, "single event must end up in DLQ");
    let entries = dlq.read_all().unwrap();
    assert_eq!(entries[0].sink, "perma");
    assert_eq!(entries[0].attempts, 1);
    assert!(entries[0].error.contains("bad config"));
}
