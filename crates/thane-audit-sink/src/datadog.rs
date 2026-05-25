//! Datadog Logs sink.
//!
//! POSTs batches to the v2 logs intake. One JSON array per batch, each entry
//! carries:
//! - the top-level Datadog fields (`ddsource`, `service`, `hostname`, `status`,
//!   `message`, `ddtags`) so the Logs UI shows useful columns out of the box;
//! - a `thane` envelope holding the full [`AuditEvent`] so downstream pipelines
//!   can pivot on any field.
//!
//! Region selection picks one of the documented intake hostnames. We do NOT
//! support arbitrary URL override here: a misrouted send hits the wrong tenant,
//! and the Datadog regions are a small closed set.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use thane_core::audit::{AuditEvent, AuditSeverity};

use crate::{AuditEventTypeKey, AuditSink, SinkError, event_type_key};

/// One of the documented Datadog Logs intake regions. Each maps to a fixed
/// intake hostname; we never accept a free-form URL because crossing regions
/// silently is a data-residency footgun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatadogRegion {
    Us1,
    Us3,
    Us5,
    Eu,
    Ap1,
}

impl DatadogRegion {
    pub fn intake_host(self) -> &'static str {
        match self {
            DatadogRegion::Us1 => "http-intake.logs.datadoghq.com",
            DatadogRegion::Us3 => "http-intake.logs.us3.datadoghq.com",
            DatadogRegion::Us5 => "http-intake.logs.us5.datadoghq.com",
            DatadogRegion::Eu => "http-intake.logs.datadoghq.eu",
            DatadogRegion::Ap1 => "http-intake.logs.ap1.datadoghq.com",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "us3" => DatadogRegion::Us3,
            "us5" => DatadogRegion::Us5,
            "eu" => DatadogRegion::Eu,
            "ap1" => DatadogRegion::Ap1,
            // "us" / "us1" / unknown → US1 (matches Datadog SDK defaults).
            _ => DatadogRegion::Us1,
        }
    }
}

impl fmt::Display for DatadogRegion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatadogRegion::Us1 => f.write_str("us"),
            DatadogRegion::Us3 => f.write_str("us3"),
            DatadogRegion::Us5 => f.write_str("us5"),
            DatadogRegion::Eu => f.write_str("eu"),
            DatadogRegion::Ap1 => f.write_str("ap1"),
        }
    }
}

/// Datadog accepts up to 1000 entries per request; the dispatcher's MAX_BATCH
/// is 100 so we never approach the limit, but we re-state it here so anyone
/// reading this code knows the contract.
pub const DATADOG_MAX_ENTRIES_PER_REQUEST: usize = 1000;

#[derive(Debug, Clone)]
pub struct DatadogConfig {
    pub region: DatadogRegion,
    pub api_key: String,
    pub env: String,
    pub service: String,
    pub hostname_override: Option<String>,
    pub timeout: Duration,
    pub min_severity: AuditSeverity,
    pub event_filter: Option<HashSet<AuditEventTypeKey>>,
    pub user_agent: String,
    /// Optional override of the intake URL. Used only by tests against a mock
    /// HTTP server; production code leaves this `None`.
    pub url_override: Option<String>,
}

impl DatadogConfig {
    pub fn new(region: DatadogRegion, api_key: impl Into<String>) -> Self {
        Self {
            region,
            api_key: api_key.into(),
            env: "prod".to_string(),
            service: "thane".to_string(),
            hostname_override: None,
            timeout: Duration::from_secs(10),
            min_severity: AuditSeverity::Info,
            event_filter: None,
            user_agent: format!("thane/{}", env!("CARGO_PKG_VERSION")),
            url_override: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct DatadogEntry<'a> {
    ddsource: &'a str,
    service: &'a str,
    hostname: &'a str,
    message: &'a str,
    status: &'static str,
    ddtags: String,
    /// Nested envelope keeps the original event verbatim for downstream
    /// pivots without us having to flatten arbitrary metadata.
    thane: &'a AuditEvent,
}

pub struct DatadogSink {
    cfg: DatadogConfig,
    client: reqwest::Client,
    hostname: String,
    url: String,
}

impl DatadogSink {
    pub fn new(cfg: DatadogConfig) -> Result<Self, SinkError> {
        if cfg.api_key.trim().is_empty() {
            return Err(SinkError::Permanent(
                "datadog API key is empty — refusing to start sink".to_string(),
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
                if h.is_empty() {
                    "thane".to_string()
                } else {
                    h
                }
            });

        let url = cfg
            .url_override
            .clone()
            .unwrap_or_else(|| format!("https://{}/api/v2/logs", cfg.region.intake_host()));

        Ok(Self { cfg, client, hostname, url })
    }

    /// Render a batch as the JSON array the Logs API expects. Public so tests
    /// can inspect the request body without spinning up a server.
    pub fn build_body(&self, batch: &[AuditEvent]) -> Vec<u8> {
        let entries: Vec<DatadogEntry<'_>> = batch
            .iter()
            .map(|ev| {
                let ddtags = build_ddtags(&self.cfg.env, ev);
                DatadogEntry {
                    ddsource: "thane",
                    service: &self.cfg.service,
                    hostname: &self.hostname,
                    message: ev.description.as_str(),
                    status: severity_to_status(ev.severity),
                    ddtags,
                    thane: ev,
                }
            })
            .collect();
        serde_json::to_vec(&entries).unwrap_or_default()
    }
}

#[async_trait]
impl AuditSink for DatadogSink {
    fn name(&self) -> &str {
        "datadog"
    }
    fn min_severity(&self) -> AuditSeverity {
        self.cfg.min_severity
    }
    fn event_filter(&self) -> Option<&HashSet<AuditEventTypeKey>> {
        self.cfg.event_filter.as_ref()
    }

    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        if batch.is_empty() {
            return Ok(());
        }
        let body = self.build_body(batch);

        let resp = self
            .client
            .post(&self.url)
            .header("DD-API-KEY", &self.cfg.api_key)
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
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            // Datadog throttles per-org; that is transient, not config error.
            return Err(SinkError::Transient(format!("HTTP {status}: {snippet}")));
        }
        if status.is_client_error() {
            return Err(SinkError::Permanent(format!("HTTP {status}: {snippet}")));
        }
        Err(SinkError::Transient(format!("HTTP {status}: {snippet}")))
    }
}

/// Map our four-level severity to the Datadog Logs `status` field.
///
/// Datadog accepts `emergency|alert|critical|error|warning|notice|info|debug`;
/// we project onto the four we routinely use, with `alert` rendered as `error`
/// (not Datadog's `alert`) because Datadog's UI treats `error` as the visible
/// "something needs attention" tier while `alert` is rarely indexed.
pub fn severity_to_status(s: AuditSeverity) -> &'static str {
    match s {
        AuditSeverity::Info => "info",
        AuditSeverity::Warning => "warning",
        AuditSeverity::Alert => "error",
        AuditSeverity::Critical => "critical",
    }
}

/// Build the Datadog tag string. Tags must use lowercase ASCII; values may
/// contain a limited charset. We sanitize values so untrusted strings (agent
/// names, custom event types) cannot inject extra tags.
fn build_ddtags(env: &str, ev: &AuditEvent) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(6);
    parts.push(format!("env:{}", sanitize_tag_value(env)));
    parts.push(format!("workspace:{}", ev.workspace_id));
    parts.push(format!("event_type:{}", event_type_key(&ev.event_type)));
    parts.push(format!("severity:{}", severity_to_status(ev.severity)));
    if let Some(agent) = ev.agent_name.as_deref()
        && !agent.is_empty()
    {
        parts.push(format!("agent:{}", sanitize_tag_value(agent)));
    }
    if let Some(user) = ev.system_user.as_deref()
        && !user.is_empty()
    {
        parts.push(format!("user:{}", sanitize_tag_value(user)));
    }
    parts.join(",")
}

/// Restrict to alphanumerics + `_-/.:` (Datadog's accepted tag-value charset).
/// Anything else is collapsed to `_`. Length-capped at 200 chars to stay under
/// Datadog's per-tag limit of 200.
fn sanitize_tag_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(200));
    for c in raw.chars().take(200) {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | ':') {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// Helper exposed for tests: parse the Datadog body back into entries.
pub fn parse_body(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::AuditEventType;
    use uuid::Uuid;

    fn ev(sev: AuditSeverity, ty: AuditEventType) -> AuditEvent {
        AuditEvent {
            id: Uuid::nil(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-05-25T10:30:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            workspace_id: Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap(),
            panel_id: None,
            event_type: ty,
            severity: sev,
            description: "something happened".into(),
            metadata: serde_json::json!({}),
            agent_name: Some("claude".into()),
            system_user: Some("alice".into()),
            system_uid: Some(501),
            prev_hash: String::new(),
            hmac: None,
        }
    }

    #[test]
    fn refuses_empty_api_key() {
        let cfg = DatadogConfig::new(DatadogRegion::Us1, "");
        let err = DatadogSink::new(cfg).err().expect("must fail");
        assert!(matches!(err, SinkError::Permanent(_)), "got {err:?}");
    }

    #[test]
    fn region_parse_round_trip() {
        for r in [
            DatadogRegion::Us1,
            DatadogRegion::Us3,
            DatadogRegion::Us5,
            DatadogRegion::Eu,
            DatadogRegion::Ap1,
        ] {
            let parsed = DatadogRegion::parse(&r.to_string());
            assert_eq!(parsed, r);
        }
        // "us1" canonicalizes to Us1.
        assert_eq!(DatadogRegion::parse("us1"), DatadogRegion::Us1);
        // Unknown defaults to Us1, not a panic.
        assert_eq!(DatadogRegion::parse("nope"), DatadogRegion::Us1);
    }

    #[test]
    fn intake_url_uses_region_host() {
        let cfg = DatadogConfig::new(DatadogRegion::Eu, "k");
        let sink = DatadogSink::new(cfg).unwrap();
        assert!(sink.url.starts_with("https://http-intake.logs.datadoghq.eu/"));
    }

    #[test]
    fn severity_maps_per_spec() {
        assert_eq!(severity_to_status(AuditSeverity::Info), "info");
        assert_eq!(severity_to_status(AuditSeverity::Warning), "warning");
        assert_eq!(severity_to_status(AuditSeverity::Alert), "error");
        assert_eq!(severity_to_status(AuditSeverity::Critical), "critical");
    }

    #[test]
    fn body_shape_matches_datadog_api() {
        let mut cfg = DatadogConfig::new(DatadogRegion::Us1, "key");
        cfg.hostname_override = Some("h1".into());
        let sink = DatadogSink::new(cfg).unwrap();
        let body = sink.build_body(&[
            ev(AuditSeverity::Info, AuditEventType::CommandExecuted),
            ev(AuditSeverity::Alert, AuditEventType::SecretAccess),
        ]);
        let arr = parse_body(&body);
        let arr = arr.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for entry in arr {
            assert_eq!(entry["ddsource"], "thane");
            assert_eq!(entry["service"], "thane");
            assert_eq!(entry["hostname"], "h1");
            assert!(entry["ddtags"].as_str().unwrap().contains("env:prod"));
            assert!(entry["ddtags"].as_str().unwrap().contains("workspace:"));
            assert!(entry["thane"].is_object());
        }
        assert_eq!(arr[0]["status"], "info");
        assert_eq!(arr[1]["status"], "error");
        assert_eq!(arr[0]["thane"]["event_type"], "command_executed");
        assert_eq!(arr[1]["thane"]["event_type"], "secret_access");
    }

    #[test]
    fn ddtags_rejects_tag_injection() {
        // Untrusted strings with commas / spaces must not split into new tags.
        let bad_ev = AuditEvent {
            agent_name: Some("evil,env:owned".into()),
            system_user: Some("alice space".into()),
            ..ev(AuditSeverity::Info, AuditEventType::CommandExecuted)
        };
        let tags = build_ddtags("prod", &bad_ev);
        // Tag count must match what we emit (env, workspace, event_type,
        // severity, agent, user = 6). Comma in agent name must not produce a
        // 7th tag.
        let tag_count = tags.split(',').count();
        assert_eq!(
            tag_count, 6,
            "tag injection must not produce extra tags; got {tags}"
        );
        // The comma inside agent_name is sanitized to _, leaving the rest
        // (colons are valid inside a Datadog tag value).
        assert!(tags.contains("agent:evil_env:owned"));
        assert!(tags.contains("user:alice_space"));
    }

    #[test]
    fn sanitize_tag_value_strips_invalid_chars() {
        assert_eq!(sanitize_tag_value("hello"), "hello");
        assert_eq!(sanitize_tag_value("Hello,World"), "hello_world");
        assert_eq!(sanitize_tag_value(""), "_");
        // 200 char cap
        let huge = "a".repeat(500);
        assert_eq!(sanitize_tag_value(&huge).len(), 200);
    }

    #[test]
    fn empty_batch_send_is_noop() {
        let cfg = DatadogConfig::new(DatadogRegion::Us1, "key");
        let sink = DatadogSink::new(cfg).unwrap();
        // build_body([]) returns "[]" which is valid JSON; verify it parses.
        let body = sink.build_body(&[]);
        let parsed = parse_body(&body);
        assert!(parsed.as_array().unwrap().is_empty());
    }
}
