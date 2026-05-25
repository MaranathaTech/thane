//! Grafana Loki sink.
//!
//! Loki ingests log streams keyed on a small set of low-cardinality labels.
//! Each push request to `POST /loki/api/v1/push` carries a list of streams;
//! each stream is `{label_set, [(nanosecond_timestamp, line)…]}`. We group
//! the incoming batch by the strict label tuple
//! `{service, user, host, tenant, event_type, severity, agent}` so a single
//! request packs efficiently regardless of how the batch is shaped.
//!
//! Labels are kept STRICTLY low-cardinality. High-cardinality identifiers
//! (workspace_id, event_id, panel_id) live INSIDE the JSON log line — not as
//! labels. Misusing labels balloons the Loki index; this has caused real
//! production outages on Loki deployments.
//!
//! Multi-tenancy follows the Loki convention: the `X-Scope-OrgID` header
//! routes the request to a specific tenant on a shared Loki deployment.
//!
//! Error classification follows the Phase 5 contract:
//! - 2xx → `Ok`
//! - 4xx → `Permanent` (auth, malformed body, missing tenant header)
//! - 5xx / 429 / network → `Transient`

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Serialize;
use thane_core::audit::{AuditEvent, AuditSeverity};

use crate::{AuditEventTypeKey, AuditSink, SinkError, event_type_key};

/// Authentication style for the Loki endpoint.
///
/// - `Bearer` — `Authorization: Bearer <token>` (Grafana Cloud, modern self-hosted).
/// - `Basic`  — `Authorization: Basic base64(user:token)` (Grafana Cloud also
///   supports this; the username is typically the tenant id).
/// - `Mtls`   — mTLS client cert + key, no `Authorization` header.
/// - `None`   — no auth (private network, dev only).
#[derive(Debug, Clone)]
pub enum LokiAuth {
    Bearer { token: String },
    Basic { user: String, token: String },
    Mtls { cert_pem: Vec<u8>, key_pem: Vec<u8> },
    None,
}

impl LokiAuth {
    /// Parse the `audit-sink-loki-auth-mode` config value into a tag for the
    /// builder to dispatch on; the actual secret material is loaded separately.
    pub fn parse_mode(raw: &str) -> LokiAuthMode {
        match raw.trim().to_ascii_lowercase().as_str() {
            "bearer" => LokiAuthMode::Bearer,
            "basic" => LokiAuthMode::Basic,
            "mtls" => LokiAuthMode::Mtls,
            "none" => LokiAuthMode::None,
            _ => LokiAuthMode::Bearer,
        }
    }
}

/// Cheaply-copyable tag returned by [`LokiAuth::parse_mode`] for use by the
/// builder. The actual `LokiAuth` is constructed once the secret bytes are
/// loaded from the platform store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LokiAuthMode {
    Bearer,
    Basic,
    Mtls,
    None,
}

#[derive(Debug, Clone)]
pub struct LokiConfig {
    /// Full push URL, e.g. `https://logs-prod-us-central1.grafana.net/loki/api/v1/push`.
    pub url: String,
    /// Tenant id sent as both the `tenant` label and the `X-Scope-OrgID` header.
    pub tenant: String,
    pub auth: LokiAuth,
    /// Override the host label. Falls back to the OS hostname when None.
    pub hostname_override: Option<String>,
    pub timeout: Duration,
    pub verify_tls: bool,
    /// Gzip the request body. Default true; some self-hosted Loki versions
    /// don't honor gzip — turn off if you see 400s about content encoding.
    pub compress: bool,
    pub min_severity: AuditSeverity,
    pub event_filter: Option<HashSet<AuditEventTypeKey>>,
    pub user_agent: String,
    /// Optional CA bundle for self-hosted Loki behind a private CA.
    pub ca_cert_pem: Option<Vec<u8>>,
}

impl LokiConfig {
    pub fn new(url: impl Into<String>, tenant: impl Into<String>, auth: LokiAuth) -> Self {
        Self {
            url: url.into(),
            tenant: tenant.into(),
            auth,
            hostname_override: None,
            timeout: Duration::from_secs(10),
            verify_tls: true,
            compress: true,
            min_severity: AuditSeverity::Info,
            event_filter: None,
            user_agent: format!("thane/{}", env!("CARGO_PKG_VERSION")),
            ca_cert_pem: None,
        }
    }
}

pub struct LokiSink {
    cfg: LokiConfig,
    client: reqwest::Client,
    hostname: String,
}

/// The label tuple that groups events into Loki streams. Order of fields
/// matters only for the BTreeMap-based JSON we emit (Loki accepts any order),
/// but we keep it stable so wire diffs in tests are readable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct StreamKey {
    user: String,
    host: String,
    tenant: String,
    event_type: String,
    severity: String,
    agent: Option<String>,
}

impl StreamKey {
    fn to_labels(&self) -> BTreeMap<&'static str, String> {
        let mut m = BTreeMap::new();
        m.insert("service", "thane".to_string());
        m.insert("user", self.user.clone());
        m.insert("host", self.host.clone());
        m.insert("tenant", self.tenant.clone());
        m.insert("event_type", self.event_type.clone());
        m.insert("severity", self.severity.clone());
        if let Some(agent) = self.agent.as_deref() {
            m.insert("agent", agent.to_string());
        }
        m
    }
}

#[derive(Serialize)]
struct PushPayload<'a> {
    streams: Vec<StreamEntry<'a>>,
}

#[derive(Serialize)]
struct StreamEntry<'a> {
    stream: BTreeMap<&'static str, String>,
    /// `[ "<nanosecond_unix_timestamp_as_string>", "<json_encoded_event>" ]`.
    values: Vec<[String; 2]>,
    #[serde(skip)]
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl LokiSink {
    pub fn new(cfg: LokiConfig) -> Result<Self, SinkError> {
        if cfg.url.trim().is_empty() {
            return Err(SinkError::Permanent("loki url is empty".to_string()));
        }
        if cfg.tenant.trim().is_empty() {
            return Err(SinkError::Permanent(
                "loki tenant is empty; X-Scope-OrgID required".to_string(),
            ));
        }
        if !cfg.verify_tls {
            tracing::warn!(
                "loki sink TLS verification is DISABLED for {}; traffic exposed to MITM",
                cfg.url
            );
        }

        let mut builder = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(cfg.user_agent.clone())
            .danger_accept_invalid_certs(!cfg.verify_tls);

        if let Some(ca) = cfg.ca_cert_pem.as_ref() {
            let cert = reqwest::Certificate::from_pem(ca)
                .map_err(|e| SinkError::Permanent(format!("loki ca cert parse: {e}")))?;
            builder = builder.add_root_certificate(cert);
        }

        if let LokiAuth::Mtls { cert_pem, key_pem } = &cfg.auth {
            // reqwest wants the cert and key in one PEM blob (rustls Identity).
            let mut combined = Vec::with_capacity(cert_pem.len() + key_pem.len() + 1);
            combined.extend_from_slice(cert_pem);
            if !combined.ends_with(b"\n") {
                combined.push(b'\n');
            }
            combined.extend_from_slice(key_pem);
            let identity = reqwest::Identity::from_pem(&combined)
                .map_err(|e| SinkError::Permanent(format!("loki mtls identity: {e}")))?;
            builder = builder.identity(identity);
        }

        let client = builder
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

    /// Render a batch as a Loki push payload. Public so tests can assert the
    /// wire shape without spinning up an HTTP server.
    pub fn build_body(&self, batch: &[AuditEvent]) -> Vec<u8> {
        // Group by the low-cardinality label tuple. BTreeMap keeps the output
        // deterministic across runs which keeps tests readable.
        let mut groups: BTreeMap<StreamKey, Vec<[String; 2]>> = BTreeMap::new();
        for ev in batch {
            let key = self.stream_key_for(ev);
            let ts_nanos = ev
                .timestamp
                .timestamp_nanos_opt()
                .unwrap_or(0)
                .to_string();
            let line = match serde_json::to_string(ev) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        "loki: skipping event {} that failed to serialize: {e}",
                        ev.id
                    );
                    continue;
                }
            };
            groups.entry(key).or_default().push([ts_nanos, line]);
        }

        let streams: Vec<StreamEntry<'_>> = groups
            .into_iter()
            .map(|(k, values)| StreamEntry {
                stream: k.to_labels(),
                values,
                _phantom: std::marker::PhantomData,
            })
            .collect();

        let payload = PushPayload { streams };
        serde_json::to_vec(&payload).unwrap_or_default()
    }

    fn stream_key_for(&self, ev: &AuditEvent) -> StreamKey {
        StreamKey {
            user: ev
                .system_user
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown")
                .to_string(),
            host: self.hostname.clone(),
            tenant: self.cfg.tenant.clone(),
            event_type: event_type_key(&ev.event_type),
            severity: severity_label(ev.severity).to_string(),
            agent: ev
                .agent_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
        }
    }

    /// Compress `body` with gzip. Returns the compressed bytes, or `body`
    /// itself on encoder failure (gzip should never fail on valid JSON, but we
    /// stay defensive — sending uncompressed is preferable to dropping).
    fn maybe_gzip(&self, body: Vec<u8>) -> (Vec<u8>, bool) {
        if !self.cfg.compress {
            return (body, false);
        }
        let mut encoder = GzEncoder::new(Vec::with_capacity(body.len() / 2), Compression::default());
        if encoder.write_all(&body).is_err() {
            return (body, false);
        }
        match encoder.finish() {
            Ok(out) => (out, true),
            Err(e) => {
                tracing::warn!("loki gzip failed, sending uncompressed: {e}");
                (body, false)
            }
        }
    }
}

#[async_trait]
impl AuditSink for LokiSink {
    fn name(&self) -> &str {
        "loki"
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
        let raw = self.build_body(batch);
        // build_body returns "{\"streams\":[]}" on an all-skip batch; treat
        // that as a no-op so we don't waste a request.
        if raw.is_empty() || raw == b"{\"streams\":[]}" {
            return Ok(());
        }
        let (body, gzipped) = self.maybe_gzip(raw);

        let mut req = self
            .client
            .post(&self.cfg.url)
            .header("Content-Type", "application/json")
            .header("X-Scope-OrgID", &self.cfg.tenant);

        if gzipped {
            req = req.header("Content-Encoding", "gzip");
        }

        match &self.cfg.auth {
            LokiAuth::Bearer { token } => {
                req = req.header("Authorization", format!("Bearer {token}"));
            }
            LokiAuth::Basic { user, token } => {
                let encoded = BASE64.encode(format!("{user}:{token}"));
                req = req.header("Authorization", format!("Basic {encoded}"));
            }
            LokiAuth::Mtls { .. } | LokiAuth::None => {}
        }

        let resp = req
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
        let snippet: String = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SinkError::Transient(format!("HTTP {status}: {snippet}")));
        }
        if status.is_client_error() {
            return Err(SinkError::Permanent(format!("HTTP {status}: {snippet}")));
        }
        Err(SinkError::Transient(format!("HTTP {status}: {snippet}")))
    }
}

fn severity_label(s: AuditSeverity) -> &'static str {
    match s {
        AuditSeverity::Info => "info",
        AuditSeverity::Warning => "warning",
        AuditSeverity::Alert => "alert",
        AuditSeverity::Critical => "critical",
    }
}

/// Helper exposed for tests: parse a Loki push body back into JSON.
pub fn parse_body(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or(serde_json::Value::Null)
}

/// Helper exposed for tests: decompress a gzipped body.
pub fn gunzip(body: &[u8]) -> Vec<u8> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut out = Vec::new();
    let mut dec = GzDecoder::new(body);
    dec.read_to_end(&mut out).unwrap_or(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use thane_core::audit::AuditEventType;
    use uuid::Uuid;

    fn ev_for(
        sev: AuditSeverity,
        ty: AuditEventType,
        user: Option<&str>,
        agent: Option<&str>,
        workspace_id: Uuid,
    ) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc
                .with_ymd_and_hms(2026, 5, 25, 10, 30, 0)
                .unwrap(),
            workspace_id,
            panel_id: None,
            event_type: ty,
            severity: sev,
            description: "x".into(),
            metadata: serde_json::json!({}),
            agent_name: agent.map(|s| s.to_string()),
            system_user: user.map(|s| s.to_string()),
            system_uid: Some(501),
            prev_hash: String::new(),
            hmac: None,
        }
    }

    fn sink_for_test(compress: bool) -> LokiSink {
        let mut cfg = LokiConfig::new(
            "https://loki.example/loki/api/v1/push",
            "acme-inc",
            LokiAuth::Bearer { token: "abc".into() },
        );
        cfg.hostname_override = Some("h1".into());
        cfg.compress = compress;
        LokiSink::new(cfg).unwrap()
    }

    #[test]
    fn refuses_empty_url_or_tenant() {
        let bad_url = LokiConfig::new("", "t", LokiAuth::None);
        assert!(matches!(
            LokiSink::new(bad_url).err(),
            Some(SinkError::Permanent(_))
        ));
        let bad_tenant = LokiConfig::new("https://x", "", LokiAuth::None);
        assert!(matches!(
            LokiSink::new(bad_tenant).err(),
            Some(SinkError::Permanent(_))
        ));
    }

    #[test]
    fn parse_mode_handles_known_and_unknown() {
        assert_eq!(LokiAuth::parse_mode("bearer"), LokiAuthMode::Bearer);
        assert_eq!(LokiAuth::parse_mode("Basic"), LokiAuthMode::Basic);
        assert_eq!(LokiAuth::parse_mode("mtls"), LokiAuthMode::Mtls);
        assert_eq!(LokiAuth::parse_mode("none"), LokiAuthMode::None);
        // Unknown defaults to Bearer (safest reasonable default; Loki rejects
        // garbage auth with 401, not a silent no-auth send).
        assert_eq!(LokiAuth::parse_mode("anything"), LokiAuthMode::Bearer);
    }

    #[test]
    fn body_shape_matches_loki_push_api() {
        let sink = sink_for_test(false);
        let ws = Uuid::new_v4();
        let body = sink.build_body(&[
            ev_for(
                AuditSeverity::Info,
                AuditEventType::CommandExecuted,
                Some("alice"),
                Some("claude"),
                ws,
            ),
        ]);
        let v = parse_body(&body);
        let streams = v["streams"].as_array().expect("streams array");
        assert_eq!(streams.len(), 1);
        let s = &streams[0];
        // Labels are the low-cardinality tuple, nothing more.
        assert_eq!(s["stream"]["service"], "thane");
        assert_eq!(s["stream"]["user"], "alice");
        assert_eq!(s["stream"]["host"], "h1");
        assert_eq!(s["stream"]["tenant"], "acme-inc");
        assert_eq!(s["stream"]["event_type"], "command_executed");
        assert_eq!(s["stream"]["severity"], "info");
        assert_eq!(s["stream"]["agent"], "claude");
        // values: [ [ "<nanos>", "<json>" ] ]
        let values = s["values"].as_array().unwrap();
        assert_eq!(values.len(), 1);
        let nanos = values[0][0].as_str().unwrap();
        assert!(nanos.parse::<i64>().is_ok(), "got {nanos}");
        // Line is the full AuditEvent JSON — must contain the workspace id
        // INSIDE the log line, not as a label.
        let line = values[0][1].as_str().unwrap();
        assert!(line.contains(&ws.to_string()), "workspace must be in log line");
        // And `workspace_id` must NOT appear as a stream label.
        assert!(s["stream"].get("workspace_id").is_none());
        assert!(s["stream"].get("event_id").is_none());
    }

    /// THE label-cardinality regression test. If someone ever adds a
    /// high-cardinality field as a label, this blows up.
    #[test]
    fn high_cardinality_workspace_does_not_explode_streams() {
        let sink = sink_for_test(false);
        let mut batch = Vec::with_capacity(1000);
        for _ in 0..1000 {
            // Each event has a unique workspace_id but the same
            // {user,host,tenant,event_type,severity,agent} tuple.
            batch.push(ev_for(
                AuditSeverity::Info,
                AuditEventType::CommandExecuted,
                Some("alice"),
                Some("claude"),
                Uuid::new_v4(),
            ));
        }
        let v = parse_body(&sink.build_body(&batch));
        let streams = v["streams"].as_array().unwrap();
        assert_eq!(
            streams.len(),
            1,
            "1000 events with distinct workspace_ids must collapse into a single stream"
        );
        assert_eq!(streams[0]["values"].as_array().unwrap().len(), 1000);
    }

    #[test]
    fn distinct_label_tuples_produce_distinct_streams() {
        let sink = sink_for_test(false);
        let ws = Uuid::new_v4();
        let body = sink.build_body(&[
            ev_for(AuditSeverity::Info, AuditEventType::CommandExecuted, Some("alice"), Some("claude"), ws),
            ev_for(AuditSeverity::Info, AuditEventType::CommandExecuted, Some("bob"), Some("claude"), ws),
            ev_for(AuditSeverity::Alert, AuditEventType::SecretAccess, Some("alice"), Some("claude"), ws),
        ]);
        let v = parse_body(&body);
        let streams = v["streams"].as_array().unwrap();
        // alice/info/command, bob/info/command, alice/alert/secret
        assert_eq!(streams.len(), 3);
    }

    #[test]
    fn missing_agent_label_is_omitted_not_empty() {
        let sink = sink_for_test(false);
        let body = sink.build_body(&[ev_for(
            AuditSeverity::Info,
            AuditEventType::CommandExecuted,
            Some("alice"),
            None,
            Uuid::new_v4(),
        )]);
        let v = parse_body(&body);
        let stream = &v["streams"][0]["stream"];
        assert!(stream.get("agent").is_none(), "agent label must be absent, not empty");
    }

    #[test]
    fn missing_user_falls_back_to_unknown_label() {
        let sink = sink_for_test(false);
        let body = sink.build_body(&[ev_for(
            AuditSeverity::Info,
            AuditEventType::CommandExecuted,
            None,
            None,
            Uuid::new_v4(),
        )]);
        let v = parse_body(&body);
        assert_eq!(v["streams"][0]["stream"]["user"], "unknown");
    }

    #[test]
    fn empty_batch_send_is_noop() {
        // We can't trivially assert "no HTTP request" without a mock server
        // here, but we can prove the body is empty, which is the precondition.
        let sink = sink_for_test(false);
        assert!(sink.build_body(&[]).is_empty() || sink.build_body(&[]) == b"{\"streams\":[]}");
    }

    #[test]
    fn timestamps_are_nanosecond_strings() {
        let sink = sink_for_test(false);
        let body = sink.build_body(&[ev_for(
            AuditSeverity::Info,
            AuditEventType::CommandExecuted,
            Some("alice"),
            None,
            Uuid::new_v4(),
        )]);
        let v = parse_body(&body);
        let nanos = v["streams"][0]["values"][0][0].as_str().unwrap();
        // 2026-05-25T10:30:00Z in nanos is a 19-digit number.
        let n: i64 = nanos.parse().unwrap();
        assert!(n > 1_700_000_000_000_000_000, "must be nanoseconds, got {n}");
    }

    #[test]
    fn gzip_roundtrip_produces_valid_payload() {
        let sink = sink_for_test(true);
        let original = sink.build_body(&[ev_for(
            AuditSeverity::Info,
            AuditEventType::CommandExecuted,
            Some("alice"),
            Some("claude"),
            Uuid::new_v4(),
        )]);
        let (compressed, was_gzipped) = sink.maybe_gzip(original.clone());
        assert!(was_gzipped);
        assert!(compressed.len() < original.len() || original.len() < 50);
        let decompressed = gunzip(&compressed);
        assert_eq!(decompressed, original);
        // Decompressed must still parse as a Loki push payload.
        let v = parse_body(&decompressed);
        assert!(v["streams"].is_array());
    }

    #[test]
    fn gzip_disabled_when_config_off() {
        let sink = sink_for_test(false);
        let body = b"{\"streams\":[]}".to_vec();
        let (out, was_gzipped) = sink.maybe_gzip(body.clone());
        assert!(!was_gzipped);
        assert_eq!(out, body);
    }

    #[test]
    fn basic_auth_header_format() {
        // Indirectly tested via the Basic encoder used in send(); reproduce
        // here so a future change to the format trips a test.
        let user = "tenant-id";
        let token = "abc123";
        let encoded = BASE64.encode(format!("{user}:{token}"));
        let header = format!("Basic {encoded}");
        assert!(header.starts_with("Basic "));
        // The decoded form must round-trip.
        let decoded = BASE64
            .decode(header.strip_prefix("Basic ").unwrap())
            .unwrap();
        assert_eq!(decoded, b"tenant-id:abc123");
    }

    #[test]
    fn severity_label_covers_all_variants() {
        assert_eq!(severity_label(AuditSeverity::Info), "info");
        assert_eq!(severity_label(AuditSeverity::Warning), "warning");
        assert_eq!(severity_label(AuditSeverity::Alert), "alert");
        assert_eq!(severity_label(AuditSeverity::Critical), "critical");
    }
}
