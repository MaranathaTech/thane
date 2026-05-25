//! External audit-log sinks for thane.
//!
//! The dispatcher is what `AuditLog::record` hands a copy of every event to.
//! Each configured sink owns its delivery loop, its retry policy, and its
//! dead-letter file. Sinks NEVER block the event-emitting code path — pushes
//! are non-blocking via `try_send` on a bounded MPSC channel, and overflowing
//! events spill straight to the DLQ.
//!
//! See `phase-5` design doc for the framing rationale.

use std::collections::HashSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use thiserror::Error;

pub mod build;
pub mod dispatcher;
pub mod dlq;
pub mod filters;
pub mod handle;
pub mod status;

#[cfg(feature = "syslog")]
pub mod syslog;

#[cfg(feature = "webhook")]
pub mod webhook;

#[cfg(feature = "splunk")]
pub mod splunk_hec;

#[cfg(feature = "datadog")]
pub mod datadog;

#[cfg(feature = "s3")]
pub mod s3;

#[cfg(feature = "loki")]
pub mod loki;

pub use build::build_dispatcher_from_config;
pub use dispatcher::{AuditDispatcher, DispatcherConfig};
pub use dlq::{DeadLetterEntry, DeadLetterQueue};
pub use handle::DispatcherHandle;
pub use status::{SinkHealth, SinkStatus, SinkStatusReport};

/// A type-stable key identifying an [`AuditEventType`] for use in sink filters.
///
/// We can't put the enum itself in a HashSet across the FFI / config boundary
/// — the `Custom(String)` variant makes that awkward. The serialized snake_case
/// name is the stable wire form and also what config files use.
pub type AuditEventTypeKey = String;

/// Convert an [`AuditEventType`] to its filter key (snake_case serialization).
pub fn event_type_key(t: &AuditEventType) -> AuditEventTypeKey {
    // Round-trips through serde to share the same snake_case the rest of the
    // codebase relies on (e.g. config files, RPC payloads).
    serde_json::to_value(t)
        .ok()
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s),
            // Custom("foo") serializes as {"custom":"foo"} — flatten to "foo".
            serde_json::Value::Object(map) => map
                .into_iter()
                .next()
                .and_then(|(k, v)| {
                    if k == "custom" {
                        v.as_str().map(|s| s.to_string())
                    } else {
                        Some(k)
                    }
                }),
            _ => None,
        })
        .unwrap_or_default()
}

/// A target for delivered audit events.
///
/// Implementations must be cheap to clone (or share via `Arc`) and safe to
/// call from a tokio task. `send` is called with batches of events; returning
/// `Ok(())` confirms the entire batch landed.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Best-effort name for status reporting (`"syslog"`, `"webhook"`, ...).
    fn name(&self) -> &str;

    /// Lowest severity this sink wants. Events below are filtered out by
    /// the dispatcher before [`AuditSink::send`] is called.
    fn min_severity(&self) -> AuditSeverity {
        AuditSeverity::Info
    }

    /// Optional whitelist of event-type keys this sink wants. `None` accepts all.
    fn event_filter(&self) -> Option<&HashSet<AuditEventTypeKey>> {
        None
    }

    /// Ship a batch of events to the external system.
    ///
    /// On `Err(SinkError::Transient)`, the dispatcher retries with backoff.
    /// On `Err(SinkError::Permanent)`, the dispatcher DLQs immediately.
    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError>;
}

/// Why a single delivery attempt failed.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum SinkError {
    /// Try again later (network blip, 5xx, connection reset, etc.).
    #[error("transient sink failure: {0}")]
    Transient(String),
    /// Don't retry — the request would always fail (4xx, bad config).
    #[error("permanent sink failure: {0}")]
    Permanent(String),
}

impl SinkError {
    pub fn is_transient(&self) -> bool {
        matches!(self, SinkError::Transient(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::AuditEventType;

    #[test]
    fn event_type_key_handles_builtin_variants() {
        assert_eq!(event_type_key(&AuditEventType::CommandExecuted), "command_executed");
        assert_eq!(event_type_key(&AuditEventType::SecretAccess), "secret_access");
        assert_eq!(event_type_key(&AuditEventType::ClaudeAppChat), "claude_app_chat");
    }

    #[test]
    fn event_type_key_handles_custom_variant() {
        let key = event_type_key(&AuditEventType::Custom("MyEvent".to_string()));
        assert_eq!(key, "MyEvent");
    }

    #[test]
    fn sink_error_classification() {
        assert!(SinkError::Transient("blip".into()).is_transient());
        assert!(!SinkError::Permanent("4xx".into()).is_transient());
    }
}
