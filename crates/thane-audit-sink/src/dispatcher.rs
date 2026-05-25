//! Audit dispatcher: bounded queue, batching, per-sink retry, DLQ on giveup.
//!
//! Lifecycle:
//! 1. Caller builds a [`DispatcherConfig`] with sinks + DLQ path.
//! 2. [`AuditDispatcher::spawn`] starts the background drain loop and returns
//!    a [`DispatcherHandle`] for `AuditLog::record` to push into.
//! 3. Caller can poll [`AuditDispatcher::status_snapshot`] for the UI.
//!
//! Design notes:
//! - One bounded MPSC channel (10k events) feeds the drain task.
//! - Drain batches up to `MAX_BATCH` events or `MAX_BATCH_INTERVAL`, whichever
//!   first, then dispatches to all sinks in parallel via `tokio::spawn`.
//! - Each per-sink dispatch retries with exponential backoff up to 5 attempts.
//! - `Permanent` errors skip retries and DLQ immediately.
//! - Stats live behind an `Arc<RwLock<SinkRuntime>>` per sink so the RPC path
//!   can read without contending with the dispatch path.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{Instant, sleep};

use thane_core::audit::AuditEvent;

use crate::dlq::DeadLetterQueue;
use crate::event_type_key;
use crate::handle::DispatcherHandle;
use crate::status::{SinkHealth, SinkStatus, SinkStatusReport};
use crate::{AuditSink, SinkError};

/// Channel capacity for events waiting to be batched.
pub const CHANNEL_CAPACITY: usize = 10_000;
/// Max events per batch shipped to one sink.
pub const MAX_BATCH: usize = 100;
/// Max delay before flushing a partial batch.
pub const MAX_BATCH_INTERVAL: Duration = Duration::from_secs(5);
/// Max retry attempts per batch per sink (counting the initial try).
pub const MAX_ATTEMPTS: u32 = 5;
/// Cap on the exponential backoff sleep.
pub const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Number of consecutive permanent errors before a sink is reported "failing".
const FAILING_PERMANENT_THRESHOLD: u32 = 3;
/// DLQ size threshold (per sink) before reporting "failing".
const FAILING_DLQ_THRESHOLD: u64 = 100;

/// Mutable runtime stats for a single sink. Lives behind an Arc<RwLock>.
struct SinkRuntime {
    name: String,
    sent_total: u64,
    last_success: Option<chrono::DateTime<Utc>>,
    last_error: Option<String>,
    /// Consecutive permanent errors since the last success — drives the
    /// "failing" health pill.
    consec_permanent: u32,
}

impl SinkRuntime {
    fn new(name: String) -> Self {
        Self {
            name,
            sent_total: 0,
            last_success: None,
            last_error: None,
            consec_permanent: 0,
        }
    }
}

/// Configuration consumed by [`AuditDispatcher::spawn`].
pub struct DispatcherConfig {
    pub sinks: Vec<Arc<dyn AuditSink>>,
    pub dlq: Arc<DeadLetterQueue>,
}

/// Manages the dispatch loop. The actual draining runs as a tokio task; this
/// struct is the read-side handle for RPC / UI.
pub struct AuditDispatcher {
    handle: DispatcherHandle,
    runtimes: Vec<Arc<RwLock<SinkRuntime>>>,
    dlq: Arc<DeadLetterQueue>,
}

impl AuditDispatcher {
    /// Spawn the dispatcher task on the ambient tokio runtime.
    ///
    /// Returns immediately. The task lives until the channel is closed (i.e.
    /// every clone of the returned handle is dropped).
    pub fn spawn(config: DispatcherConfig) -> Self {
        let (tx, rx) = mpsc::channel::<AuditEvent>(CHANNEL_CAPACITY);
        let runtimes: Vec<Arc<RwLock<SinkRuntime>>> = config
            .sinks
            .iter()
            .map(|s| Arc::new(RwLock::new(SinkRuntime::new(s.name().to_string()))))
            .collect();

        let handle = DispatcherHandle::new(tx, config.dlq.clone());

        let task_state = TaskState {
            sinks: config.sinks.clone(),
            runtimes: runtimes.clone(),
            dlq: config.dlq.clone(),
        };
        tokio::spawn(drain_loop(rx, task_state));

        Self {
            handle,
            runtimes,
            dlq: config.dlq,
        }
    }

    /// Cheaply clonable handle for `AuditLog::record`.
    pub fn handle(&self) -> DispatcherHandle {
        self.handle.clone()
    }

    /// Snapshot every sink's current state for `audit.sink_status`.
    pub async fn status_snapshot(&self) -> SinkStatusReport {
        let queued = self.handle.queued();
        let mut sinks = Vec::with_capacity(self.runtimes.len());
        for rt in &self.runtimes {
            let r = rt.read().await;
            let dlq_total = self.dlq.count_for_sink(&r.name);
            sinks.push(SinkStatus {
                name: r.name.clone(),
                enabled: true,
                queued,
                sent_total: r.sent_total,
                dlq_total,
                last_success: r.last_success,
                last_error: r.last_error.clone(),
                status: classify_health(r.consec_permanent, dlq_total),
            });
        }
        SinkStatusReport { sinks }
    }

    /// Synchronous (non-async) variant of [`status_snapshot`] for callers that
    /// can't await — uses `blocking_read` on the same locks.
    pub fn status_snapshot_blocking(&self) -> SinkStatusReport {
        let queued = self.handle.queued();
        let mut sinks = Vec::with_capacity(self.runtimes.len());
        for rt in &self.runtimes {
            let r = rt.blocking_read();
            let dlq_total = self.dlq.count_for_sink(&r.name);
            sinks.push(SinkStatus {
                name: r.name.clone(),
                enabled: true,
                queued,
                sent_total: r.sent_total,
                dlq_total,
                last_success: r.last_success,
                last_error: r.last_error.clone(),
                status: classify_health(r.consec_permanent, dlq_total),
            });
        }
        SinkStatusReport { sinks }
    }

    /// Re-enqueue a previously DLQ'd event back through the dispatcher.
    /// Used by `thane-cli audit dlq retry`.
    pub fn retry_event(&self, event: &AuditEvent) {
        self.handle.try_send(event);
    }
}

/// State the drain task needs. Moved into the spawned task.
struct TaskState {
    sinks: Vec<Arc<dyn AuditSink>>,
    runtimes: Vec<Arc<RwLock<SinkRuntime>>>,
    dlq: Arc<DeadLetterQueue>,
}

async fn drain_loop(mut rx: mpsc::Receiver<AuditEvent>, state: TaskState) {
    let mut batch: Vec<AuditEvent> = Vec::with_capacity(MAX_BATCH);
    let mut batch_deadline = Instant::now() + MAX_BATCH_INTERVAL;

    loop {
        let now = Instant::now();
        let until_deadline = batch_deadline.saturating_duration_since(now);

        // If we already have a full batch, flush right away.
        if batch.len() >= MAX_BATCH {
            dispatch_batch(&state, &batch).await;
            batch.clear();
            batch_deadline = Instant::now() + MAX_BATCH_INTERVAL;
            continue;
        }

        tokio::select! {
            maybe = rx.recv() => {
                match maybe {
                    Some(ev) => {
                        if batch.is_empty() {
                            batch_deadline = Instant::now() + MAX_BATCH_INTERVAL;
                        }
                        batch.push(ev);
                    }
                    None => {
                        // Channel closed: flush remainder and exit.
                        if !batch.is_empty() {
                            dispatch_batch(&state, &batch).await;
                        }
                        tracing::debug!("audit dispatcher draining: channel closed");
                        return;
                    }
                }
            }
            _ = sleep(until_deadline), if !batch.is_empty() => {
                dispatch_batch(&state, &batch).await;
                batch.clear();
                batch_deadline = Instant::now() + MAX_BATCH_INTERVAL;
            }
        }
    }
}

/// Filter `batch` per sink and dispatch all sinks in parallel.
async fn dispatch_batch(state: &TaskState, batch: &[AuditEvent]) {
    let mut tasks = Vec::with_capacity(state.sinks.len());
    for (sink, runtime) in state.sinks.iter().zip(state.runtimes.iter()) {
        let sink = sink.clone();
        let runtime = runtime.clone();
        let dlq = state.dlq.clone();
        let filtered = filter_for_sink(sink.as_ref(), batch);
        if filtered.is_empty() {
            continue;
        }
        tasks.push(tokio::spawn(async move {
            send_with_retry(sink.as_ref(), &filtered, &runtime, &dlq).await;
        }));
    }
    for t in tasks {
        if let Err(e) = t.await {
            tracing::error!("audit sink task panicked: {e}");
        }
    }
}

fn filter_for_sink(sink: &dyn AuditSink, batch: &[AuditEvent]) -> Vec<AuditEvent> {
    let min_sev = sink.min_severity();
    let allowed: Option<&_> = sink.event_filter();
    batch
        .iter()
        .filter(|e| e.severity >= min_sev)
        .filter(|e| match allowed {
            Some(set) => set.contains(&event_type_key(&e.event_type)),
            None => true,
        })
        .cloned()
        .collect()
}

/// Send a batch with retries. On final failure or any `Permanent` error,
/// each event in the batch lands in the DLQ.
async fn send_with_retry(
    sink: &dyn AuditSink,
    batch: &[AuditEvent],
    runtime: &RwLock<SinkRuntime>,
    dlq: &DeadLetterQueue,
) {
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        match sink.send(batch).await {
            Ok(()) => {
                let mut r = runtime.write().await;
                r.sent_total = r.sent_total.saturating_add(batch.len() as u64);
                r.last_success = Some(Utc::now());
                r.last_error = None;
                r.consec_permanent = 0;
                return;
            }
            Err(SinkError::Permanent(msg)) => {
                tracing::warn!(
                    "sink {} permanent failure on attempt {}: {msg}",
                    sink.name(),
                    attempt
                );
                {
                    let mut r = runtime.write().await;
                    r.last_error = Some(format!("permanent: {msg}"));
                    r.consec_permanent = r.consec_permanent.saturating_add(1);
                }
                for ev in batch {
                    dlq.write_failure(sink.name(), &msg, attempt, ev);
                }
                return;
            }
            Err(SinkError::Transient(msg)) => {
                tracing::warn!(
                    "sink {} transient failure on attempt {}: {msg}",
                    sink.name(),
                    attempt
                );
                {
                    let mut r = runtime.write().await;
                    r.last_error = Some(format!("transient: {msg}"));
                }
                if attempt >= MAX_ATTEMPTS {
                    for ev in batch {
                        dlq.write_failure(
                            sink.name(),
                            &format!("transient (final): {msg}"),
                            attempt,
                            ev,
                        );
                    }
                    return;
                }
                sleep(backoff_for(attempt)).await;
            }
        }
    }
}

/// Exponential backoff: 1s, 2s, 4s, 8s, ... capped at MAX_BACKOFF.
pub fn backoff_for(attempt: u32) -> Duration {
    let base = 1u64 << (attempt.saturating_sub(1).min(10));
    let secs = base.min(MAX_BACKOFF.as_secs());
    Duration::from_secs(secs)
}

fn classify_health(consec_permanent: u32, dlq_total: u64) -> SinkHealth {
    if consec_permanent >= FAILING_PERMANENT_THRESHOLD || dlq_total >= FAILING_DLQ_THRESHOLD {
        SinkHealth::Failing
    } else if consec_permanent > 0 || dlq_total > 0 {
        SinkHealth::Degraded
    } else {
        SinkHealth::Healthy
    }
}

/// Convenience: build a `DispatcherHandle` that does NOTHING (no sinks).
///
/// Useful in unit tests for code that wants to plumb a handle but isn't
/// testing delivery semantics, and for processes (GTK/macOS bridge) that
/// don't enable any sinks.
pub fn noop_handle(audit_dir: impl Into<std::path::PathBuf>) -> DispatcherHandle {
    let dlq = Arc::new(DeadLetterQueue::new(audit_dir));
    // Build a channel and immediately drop the receiver so try_send always
    // hits the "closed" path and spills to DLQ. To avoid that, we instead
    // keep the receiver alive but never read it — bounded channel will
    // eventually fill, then spill. For a true no-op, we want neither: build
    // a channel with an empty draining task.
    let (tx, mut rx) = mpsc::channel::<AuditEvent>(CHANNEL_CAPACITY);
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            // drop
        }
    });
    DispatcherHandle::new(tx, dlq)
}

/// Statistics about the channel for UI hover-text. Kept here so consumers
/// don't have to peek at MPSC internals.
pub fn channel_stats(handle: &DispatcherHandle) -> HashMap<&'static str, usize> {
    let mut m = HashMap::new();
    m.insert("queued", handle.queued());
    m.insert("capacity", handle.capacity());
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuditSink;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};
    use thane_core::audit::{AuditEventType, AuditSeverity};
    use uuid::Uuid;

    fn fake_event(sev: AuditSeverity, ty: AuditEventType, desc: &str) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            workspace_id: Uuid::new_v4(),
            panel_id: None,
            event_type: ty,
            severity: sev,
            description: desc.to_string(),
            metadata: serde_json::json!({}),
            agent_name: None,
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        }
    }

    struct CountingSink {
        name: &'static str,
        count: Arc<AtomicU32>,
        min_sev: AuditSeverity,
    }

    #[async_trait]
    impl AuditSink for CountingSink {
        fn name(&self) -> &str { self.name }
        fn min_severity(&self) -> AuditSeverity { self.min_sev }
        async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
            self.count.fetch_add(batch.len() as u32, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff_for(1), Duration::from_secs(1));
        assert_eq!(backoff_for(2), Duration::from_secs(2));
        assert_eq!(backoff_for(3), Duration::from_secs(4));
        assert_eq!(backoff_for(4), Duration::from_secs(8));
        // Eventually caps at MAX_BACKOFF.
        assert!(backoff_for(100) <= MAX_BACKOFF);
    }

    #[test]
    fn classify_health_thresholds() {
        assert_eq!(classify_health(0, 0), SinkHealth::Healthy);
        assert_eq!(classify_health(1, 0), SinkHealth::Degraded);
        assert_eq!(classify_health(0, 1), SinkHealth::Degraded);
        assert_eq!(classify_health(FAILING_PERMANENT_THRESHOLD, 0), SinkHealth::Failing);
        assert_eq!(classify_health(0, FAILING_DLQ_THRESHOLD), SinkHealth::Failing);
    }

    #[tokio::test]
    async fn filter_for_sink_drops_below_min_severity() {
        let sink = CountingSink {
            name: "high",
            count: Arc::new(AtomicU32::new(0)),
            min_sev: AuditSeverity::Alert,
        };
        let batch = vec![
            fake_event(AuditSeverity::Info, AuditEventType::CommandExecuted, "lo"),
            fake_event(AuditSeverity::Alert, AuditEventType::SecretAccess, "hi"),
        ];
        let filtered = filter_for_sink(&sink, &batch);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].description, "hi");
    }
}
