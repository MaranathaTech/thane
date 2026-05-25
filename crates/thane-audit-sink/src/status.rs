//! Per-sink status snapshot surfaced to the UI and to `audit.sink_status` RPC.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Coarse health classification used to color the status pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkHealth {
    /// All recent deliveries succeeded.
    Healthy,
    /// Recent transient failures, but events still flowing.
    Degraded,
    /// Consecutive permanent errors or a large DLQ — operator attention needed.
    Failing,
}

/// Current state of one configured sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkStatus {
    pub name: String,
    pub enabled: bool,
    /// Events sitting in the dispatcher queue waiting to be sent to this sink.
    pub queued: usize,
    pub sent_total: u64,
    pub dlq_total: u64,
    pub last_success: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub status: SinkHealth,
}

/// Aggregate of all sinks the dispatcher knows about. Returned by RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkStatusReport {
    pub sinks: Vec<SinkStatus>,
}
