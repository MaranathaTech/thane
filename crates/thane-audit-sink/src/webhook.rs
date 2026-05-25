//! HTTPS webhook sink.
//!
//! POSTs JSON to a configured URL with an HMAC-SHA256 signature in the
//! `X-Thane-Signature` header (Stripe-style `t=<ts>,v1=<hex>`). The signed
//! payload is `<unix_ts>.<body>` to defeat replays after the timestamp
//! becomes stale.
//!
//! Error classification mirrors common SIEM gateways:
//! - 2xx → `Ok`
//! - 4xx → `Permanent` (client misconfiguration — retries will keep failing)
//! - 5xx + network errors → `Transient` (retry under backoff)

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use thane_core::audit::{AuditEvent, AuditSeverity};

use crate::{AuditEventTypeKey, AuditSink, SinkError};

type HmacSha256 = Hmac<Sha256>;

/// Wire schema version. Bump whenever the body layout changes.
pub const WEBHOOK_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub url: String,
    pub secret: Vec<u8>,
    pub timeout: Duration,
    pub min_severity: AuditSeverity,
    pub event_filter: Option<HashSet<AuditEventTypeKey>>,
    pub hostname_override: Option<String>,
    pub user_agent: String,
}

impl WebhookConfig {
    pub fn new(url: impl Into<String>, secret: Vec<u8>) -> Self {
        Self {
            url: url.into(),
            secret,
            timeout: Duration::from_secs(10),
            min_severity: AuditSeverity::Info,
            event_filter: None,
            hostname_override: None,
            user_agent: format!("thane/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[derive(Debug, Serialize)]
struct WebhookBody<'a> {
    schema_version: &'static str,
    host: &'a str,
    batch: &'a [AuditEvent],
}

pub struct WebhookSink {
    cfg: WebhookConfig,
    client: reqwest::Client,
    hostname: String,
}

impl WebhookSink {
    pub fn new(cfg: WebhookConfig) -> Result<Self, SinkError> {
        if cfg.secret.is_empty() {
            return Err(SinkError::Permanent(
                "webhook secret is empty — refusing to start sink".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(cfg.user_agent.clone())
            .build()
            .map_err(|e| SinkError::Permanent(format!("reqwest client build: {e}")))?;

        let hostname = cfg
            .hostname_override
            .clone()
            .unwrap_or_else(|| {
                let h = whoami::fallible::hostname().unwrap_or_default();
                if h.is_empty() { "thane".to_string() } else { h }
            });

        Ok(Self { cfg, client, hostname })
    }

    /// Build the body bytes and the matching signature header for a batch.
    /// Pulled out so tests can verify the HMAC without launching reqwest.
    pub fn build_signed_request(
        &self,
        batch: &[AuditEvent],
        unix_ts: u64,
    ) -> (Vec<u8>, String) {
        let body = WebhookBody {
            schema_version: WEBHOOK_SCHEMA_VERSION,
            host: &self.hostname,
            batch,
        };
        let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
        let sig = sign_payload(&self.cfg.secret, unix_ts, &body_bytes);
        let header = format!("t={unix_ts},v1={sig}");
        (body_bytes, header)
    }
}

#[async_trait]
impl AuditSink for WebhookSink {
    fn name(&self) -> &str { "webhook" }
    fn min_severity(&self) -> AuditSeverity { self.cfg.min_severity }
    fn event_filter(&self) -> Option<&HashSet<AuditEventTypeKey>> {
        self.cfg.event_filter.as_ref()
    }

    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        let unix_ts = chrono::Utc::now().timestamp() as u64;
        let (body_bytes, sig_header) = self.build_signed_request(batch, unix_ts);

        let resp = self
            .client
            .post(&self.cfg.url)
            .header("Content-Type", "application/json")
            .header("X-Thane-Signature", sig_header)
            .body(body_bytes)
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
        if status.is_client_error() {
            let body_snippet = resp.text().await.unwrap_or_default();
            let snippet: String = body_snippet.chars().take(200).collect();
            return Err(SinkError::Permanent(format!(
                "HTTP {status}: {snippet}"
            )));
        }
        let body_snippet = resp.text().await.unwrap_or_default();
        let snippet: String = body_snippet.chars().take(200).collect();
        Err(SinkError::Transient(format!(
            "HTTP {status}: {snippet}"
        )))
    }
}

/// Compute the Stripe-style HMAC: `HEX(HMAC-SHA256(secret, "<ts>.<body>"))`.
pub fn sign_payload(secret: &[u8], unix_ts: u64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(unix_ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Parse the `X-Thane-Signature` header value into `(unix_ts, hex_signature)`.
/// Returns `None` on any malformed input. Receivers use this to verify.
pub fn parse_signature_header(header: &str) -> Option<(u64, String)> {
    let mut ts: Option<u64> = None;
    let mut sig: Option<String> = None;
    for part in header.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("t=") {
            ts = rest.parse().ok();
        } else if let Some(rest) = part.strip_prefix("v1=") {
            sig = Some(rest.to_string());
        }
    }
    Some((ts?, sig?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::{AuditEventType};
    use uuid::Uuid;

    fn fake_event() -> AuditEvent {
        AuditEvent {
            id: Uuid::nil(),
            timestamp: chrono::Utc::now(),
            workspace_id: Uuid::nil(),
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

    #[test]
    fn sign_payload_is_deterministic() {
        let a = sign_payload(b"secret", 1000, b"body");
        let b = sign_payload(b"secret", 1000, b"body");
        assert_eq!(a, b);
        // Length: HMAC-SHA256 hex = 64 chars.
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn sign_payload_differs_by_ts() {
        let a = sign_payload(b"secret", 1000, b"body");
        let b = sign_payload(b"secret", 1001, b"body");
        assert_ne!(a, b);
    }

    #[test]
    fn sign_payload_differs_by_secret() {
        let a = sign_payload(b"s1", 1000, b"body");
        let b = sign_payload(b"s2", 1000, b"body");
        assert_ne!(a, b);
    }

    #[test]
    fn parse_signature_header_round_trip() {
        let h = "t=1234567890,v1=deadbeef";
        let (ts, sig) = parse_signature_header(h).unwrap();
        assert_eq!(ts, 1234567890);
        assert_eq!(sig, "deadbeef");
    }

    #[test]
    fn parse_signature_header_rejects_partial() {
        assert!(parse_signature_header("v1=deadbeef").is_none());
        assert!(parse_signature_header("t=42").is_none());
        assert!(parse_signature_header("garbage").is_none());
    }

    #[test]
    fn webhook_sink_refuses_empty_secret() {
        let cfg = WebhookConfig::new("https://example.com/x", vec![]);
        match WebhookSink::new(cfg) {
            Err(SinkError::Permanent(_)) => {}
            Err(other) => panic!("expected Permanent error, got {other:?}"),
            Ok(_) => panic!("expected error for empty secret"),
        }
    }

    #[test]
    fn build_signed_request_signature_verifies() {
        let cfg = WebhookConfig::new("https://example.com/x", b"mysecret".to_vec());
        let sink = WebhookSink::new(cfg).unwrap();
        let batch = [fake_event()];
        let ts = 1700000000u64;
        let (body, header) = sink.build_signed_request(&batch, ts);

        let (parsed_ts, parsed_sig) = parse_signature_header(&header).unwrap();
        assert_eq!(parsed_ts, ts);
        let expected = sign_payload(b"mysecret", ts, &body);
        assert_eq!(parsed_sig, expected);
    }
}
