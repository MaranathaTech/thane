//! A sink with an event_filter only receives events whose type is in the set.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use thane_audit_sink::{
    AuditDispatcher, AuditEventTypeKey, AuditSink, DispatcherConfig, DeadLetterQueue, SinkError,
};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use uuid::Uuid;

struct PickySink {
    filter: HashSet<AuditEventTypeKey>,
    received: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl AuditSink for PickySink {
    fn name(&self) -> &str { "picky" }
    fn event_filter(&self) -> Option<&HashSet<AuditEventTypeKey>> {
        Some(&self.filter)
    }
    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        let mut received = self.received.lock().unwrap();
        for ev in batch {
            let key = thane_audit_sink::event_type_key(&ev.event_type);
            received.push(key);
        }
        Ok(())
    }
}

fn ev(ty: AuditEventType) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        workspace_id: Uuid::new_v4(),
        panel_id: None,
        event_type: ty,
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

/// Wait up to `timeout` for `predicate` to return true, polling every 25ms.
async fn wait_until<F: Fn() -> bool>(timeout: Duration, predicate: F) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if predicate() { return; }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picky_sink_only_sees_allowed_types() {
    let dir = tempfile::tempdir().unwrap();
    let received: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut filter: HashSet<AuditEventTypeKey> = HashSet::new();
    filter.insert("secret_access".to_string());
    filter.insert("private_key_access".to_string());

    let dispatcher = AuditDispatcher::spawn(DispatcherConfig {
        sinks: vec![Arc::new(PickySink { filter, received: received.clone() })],
        dlq: Arc::new(DeadLetterQueue::new(dir.path())),
    });
    let handle = dispatcher.handle();

    handle.try_send(&ev(AuditEventType::CommandExecuted));
    handle.try_send(&ev(AuditEventType::SecretAccess));
    handle.try_send(&ev(AuditEventType::FileRead));
    handle.try_send(&ev(AuditEventType::PrivateKeyAccess));

    // The dispatcher flushes a partial batch after MAX_BATCH_INTERVAL (5s).
    // Wait up to 7s for the two filtered events to arrive.
    wait_until(Duration::from_secs(7), || received.lock().unwrap().len() >= 2).await;

    let got = received.lock().unwrap();
    let mut sorted: Vec<&String> = got.iter().collect();
    sorted.sort();
    assert_eq!(sorted, vec!["private_key_access", "secret_access"]);
}
