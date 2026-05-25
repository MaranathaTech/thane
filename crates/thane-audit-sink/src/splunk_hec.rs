//! Splunk HTTP Event Collector sink.
//!
//! Wire format: newline-delimited JSON per the HEC `/services/collector/event`
//! contract. One HEC event per audit event. The `event` field carries the full
//! [`AuditEvent`] JSON so downstream Splunk searches can pivot on any field
//! without us re-encoding subsets here.
//!
//! Error classification:
//! - 2xx → `Ok`
//! - 4xx → `Permanent` (auth, bad index, malformed body)
//! - 5xx + network → `Transient` (retry under dispatcher backoff)
//!
//! TLS verification is on by default. `verify_tls = false` is supported for
//! self-signed Splunk deployments but logs a `warn!` at construction so the
//! choice is auditable.

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use thane_core::audit::{AuditEvent, AuditSeverity};

use crate::{AuditEventTypeKey, AuditSink, SinkError};

/// Configuration for [`SplunkHecSink`].
#[derive(Debug, Clone)]
pub struct SplunkHecConfig {
    /// Full HEC endpoint URL, e.g.
    /// `https://splunk.example.com:8088/services/collector/event`.
    pub url: String,
    /// HEC token. Sent as `Authorization: Splunk <token>`.
    pub token: String,
    /// Optional Splunk index override. Empty / `None` lets the token's default
    /// index apply.
    pub index: Option<String>,
    /// `source` field on every HEC event (typically `"thane"`).
    pub source: String,
    /// `sourcetype` field on every HEC event. Defaults to `"thane:audit"`.
    pub sourcetype: String,
    /// `host` field. Falls back to the OS hostname when None.
    pub hostname_override: Option<String>,
    /// Verify the server TLS cert. Off only for self-signed dev instances.
    pub verify_tls: bool,
    pub timeout: Duration,
    pub min_severity: AuditSeverity,
    pub event_filter: Option<HashSet<AuditEventTypeKey>>,
    pub user_agent: String,
}

impl SplunkHecConfig {
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: token.into(),
            index: None,
            source: "thane".to_string(),
            sourcetype: "thane:audit".to_string(),
            hostname_override: None,
            verify_tls: true,
            timeout: Duration::from_secs(10),
            min_severity: AuditSeverity::Info,
            event_filter: None,
            user_agent: format!("thane/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// One HEC envelope wrapping a single audit event.
#[derive(Debug, Serialize)]
struct HecEnvelope<'a> {
    /// Epoch seconds with fractional component, matching `event.timestamp`.
    time: f64,
    host: &'a str,
    source: &'a str,
    sourcetype: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<&'a str>,
    event: &'a AuditEvent,
}

pub struct SplunkHecSink {
    cfg: SplunkHecConfig,
    client: reqwest::Client,
    hostname: String,
}

impl SplunkHecSink {
    pub fn new(cfg: SplunkHecConfig) -> Result<Self, SinkError> {
        if cfg.token.trim().is_empty() {
            return Err(SinkError::Permanent(
                "splunk HEC token is empty — refusing to start sink".to_string(),
            ));
        }
        if !cfg.verify_tls {
            tracing::warn!(
                "splunk HEC sink TLS verification is DISABLED for {}; \
                 traffic is exposed to MITM",
                cfg.url
            );
        }
        let client = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(cfg.user_agent.clone())
            .danger_accept_invalid_certs(!cfg.verify_tls)
            .build()
            .map_err(|e| SinkError::Permanent(format!("reqwest client build: {e}")))?;

        let hostname = cfg
            .hostname_override
            .clone()
            .unwrap_or_else(|| {
                let h = whoami::fallible::hostname().unwrap_or_default();
                if h.is_empty() {
                    "thane".to_string()
                } else {
                    h
                }
            });

        Ok(Self { cfg, client, hostname })
    }

    /// Render a batch as newline-delimited HEC envelopes. Public so tests can
    /// assert the wire shape without spinning up an HTTP server.
    pub fn build_body(&self, batch: &[AuditEvent]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(batch.len() * 512);
        for ev in batch {
            let env = HecEnvelope {
                time: timestamp_epoch_seconds(ev),
                host: &self.hostname,
                source: &self.cfg.source,
                sourcetype: &self.cfg.sourcetype,
                index: self.cfg.index.as_deref(),
                event: ev,
            };
            // serde_json::to_writer would be marginally cheaper but the explicit
            // newline join is easier to reason about.
            match serde_json::to_string(&env) {
                Ok(s) => {
                    buf.extend_from_slice(s.as_bytes());
                    buf.push(b'\n');
                }
                Err(e) => {
                    tracing::warn!(
                        "splunk: skipping event {} that failed to serialize: {e}",
                        ev.id
                    );
                }
            }
        }
        buf
    }
}

#[async_trait]
impl AuditSink for SplunkHecSink {
    fn name(&self) -> &str {
        "splunk"
    }
    fn min_severity(&self) -> AuditSeverity {
        self.cfg.min_severity
    }
    fn event_filter(&self) -> Option<&HashSet<AuditEventTypeKey>> {
        self.cfg.event_filter.as_ref()
    }

    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        let body = self.build_body(batch);
        if body.is_empty() {
            return Ok(());
        }

        let resp = self
            .client
            .post(&self.cfg.url)
            .header("Authorization", format!("Splunk {}", self.cfg.token))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() || e.is_connect() {
                    SinkError::Transient(format!("network: {e}"))
                } else {
                    SinkError::Transient(format!("send: {e}"))
                }
            })?;

        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let snippet: String = resp.text().await.unwrap_or_default().chars().take(200).collect();
        if status.is_client_error() {
            // HEC returns 401/403 for bad token, 400 for malformed body, 404 for
            // disabled collector: all are operator-actionable, never retry.
            return Err(SinkError::Permanent(format!("HTTP {status}: {snippet}")));
        }
        Err(SinkError::Transient(format!("HTTP {status}: {snippet}")))
    }
}

/// Convert an audit event timestamp into HEC's fractional-second epoch form.
fn timestamp_epoch_seconds(ev: &AuditEvent) -> f64 {
    let nanos = ev.timestamp.timestamp_nanos_opt().unwrap_or(0);
    nanos as f64 / 1_000_000_000.0
}

/// Helper exposed for tests: pretty-decode the HEC body back into the list of
/// envelopes (as `serde_json::Value`).
pub fn parse_ndjson_body(body: &[u8]) -> Vec<Value> {
    std::str::from_utf8(body)
        .unwrap_or("")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::AuditEventType;
    use uuid::Uuid;

    fn ev(sev: AuditSeverity) -> AuditEvent {
        AuditEvent {
            id: Uuid::nil(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-05-25T10:30:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            workspace_id: Uuid::nil(),
            panel_id: None,
            event_type: AuditEventType::SecretAccess,
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

    #[test]
    fn refuses_empty_token() {
        let cfg = SplunkHecConfig::new("https://x", "");
        let err = SplunkHecSink::new(cfg).err().expect("must fail");
        assert!(matches!(err, SinkError::Permanent(_)), "got {err:?}");
    }

    #[test]
    fn build_body_emits_one_ndjson_line_per_event() {
        let mut cfg = SplunkHecConfig::new("https://x", "tok");
        cfg.hostname_override = Some("h".into());
        cfg.index = Some("audit".into());
        let sink = SplunkHecSink::new(cfg).unwrap();
        let body = sink.build_body(&[ev(AuditSeverity::Info), ev(AuditSeverity::Alert)]);

        let envs = parse_ndjson_body(&body);
        assert_eq!(envs.len(), 2);
        assert_eq!(envs[0]["host"], "h");
        assert_eq!(envs[0]["source"], "thane");
        assert_eq!(envs[0]["sourcetype"], "thane:audit");
        assert_eq!(envs[0]["index"], "audit");
        // Event payload is the full AuditEvent struct.
        assert_eq!(envs[0]["event"]["event_type"], "secret_access");
        assert_eq!(envs[0]["event"]["severity"], "info");
        assert_eq!(envs[1]["event"]["severity"], "alert");
    }

    #[test]
    fn build_body_omits_index_when_unset() {
        let mut cfg = SplunkHecConfig::new("https://x", "tok");
        cfg.hostname_override = Some("h".into());
        let sink = SplunkHecSink::new(cfg).unwrap();
        let body = sink.build_body(&[ev(AuditSeverity::Info)]);
        let envs = parse_ndjson_body(&body);
        assert!(envs[0].get("index").is_none(), "no index when unconfigured");
    }

    #[test]
    fn timestamp_epoch_seconds_matches_chrono() {
        // 2026-05-25T10:30:00Z → epoch precomputed.
        let e = ev(AuditSeverity::Info);
        let expected = e.timestamp.timestamp() as f64;
        let got = timestamp_epoch_seconds(&e);
        // Same to the second; ev() picks an exact second.
        assert!((got - expected).abs() < 1e-6);
    }

    #[test]
    fn build_body_empty_batch_returns_empty_bytes() {
        let cfg = SplunkHecConfig::new("https://x", "tok");
        let sink = SplunkHecSink::new(cfg).unwrap();
        assert!(sink.build_body(&[]).is_empty());
    }
}
