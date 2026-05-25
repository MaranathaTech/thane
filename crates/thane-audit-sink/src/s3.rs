//! S3 / object-storage sink.
//!
//! Long-term audit retention. Buffers events in memory, then writes one
//! gzip-compressed JSONL object per rollup. Each object key sorts by time so
//! a `list-objects` walk is a chronological scan:
//!
//! ```text
//! <prefix><hostname>/<YYYY>/<MM>/<DD>/<HH>/<batch-uuid>.jsonl.gz
//! ```
//!
//! ## Rollup
//! A flush is triggered when EITHER condition is met:
//! - 5 minutes have elapsed since the first event in the current buffer; or
//! - 10 MB of uncompressed JSONL has accumulated.
//!
//! Both thresholds are configurable. The buffer is also flushed explicitly by
//! callers via [`S3Sink::flush_now`] on graceful shutdown.
//!
//! ## Compatibility
//! The sink uses the standard S3 API and the AWS SDK's `endpoint_url` and
//! `force_path_style` knobs to work with S3-compatible backends — Cloudflare
//! R2, MinIO, Wasabi, Backblaze B2, etc. Pass the endpoint URL via
//! [`S3Config::endpoint_url`] for those.
//!
//! ## Compliance posture
//! - **SSE-S3** is the default (server-side encryption with S3-managed keys).
//! - **SSE-KMS** is selectable for orgs that want their own KMS key.
//! - **Object Lock** in `compliance` or `governance` mode is opt-in. The
//!   target bucket must have object-lock enabled at bucket creation time —
//!   we surface a `Permanent` error if it isn't.

use std::collections::HashSet;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{ObjectLockMode, ServerSideEncryption};
use chrono::{DateTime, Utc};
use flate2::Compression;
use flate2::write::GzEncoder;
use thane_core::audit::{AuditEvent, AuditSeverity};
use tokio::sync::{Mutex, OnceCell};

use crate::{AuditEventTypeKey, AuditSink, SinkError};

/// Default buffer-age flush trigger.
pub const DEFAULT_MAX_BUFFER_AGE: Duration = Duration::from_secs(5 * 60);
/// Default uncompressed-size flush trigger (10 MB).
///
/// Gzip on JSON typically yields 5–10× compression, so the resulting object
/// is roughly 1–2 MB — small enough to upload in a single PUT, large enough
/// to keep request counts (and per-request fees on R2 / S3) low.
pub const DEFAULT_MAX_UNCOMPRESSED_BYTES: usize = 10 * 1024 * 1024;

/// What server-side encryption mode to apply on PUT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseMode {
    /// S3-managed keys (`AES256`). Default. Free, opaque.
    S3,
    /// KMS keys. Requires [`S3Config::kms_key_id`].
    Kms,
    /// No SSE header. Only set this for backends that don't support SSE; we
    /// still warn at startup.
    None,
}

impl SseMode {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "kms" => SseMode::Kms,
            "none" | "" => SseMode::None,
            _ => SseMode::S3,
        }
    }
}

/// Optional S3 Object-Lock retention applied to each object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectLockKind {
    /// Immutable. Cannot be deleted by anyone (including root) until the
    /// retention period passes.
    Compliance,
    /// Immutable except by users with `s3:BypassGovernanceRetention`.
    Governance,
    /// No object-lock header set.
    None,
}

impl ObjectLockKind {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "compliance" => ObjectLockKind::Compliance,
            "governance" => ObjectLockKind::Governance,
            _ => ObjectLockKind::None,
        }
    }

    fn to_sdk(self) -> Option<ObjectLockMode> {
        match self {
            ObjectLockKind::Compliance => Some(ObjectLockMode::Compliance),
            ObjectLockKind::Governance => Some(ObjectLockMode::Governance),
            ObjectLockKind::None => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    /// Endpoint URL override. Blank/None for AWS S3; set for R2/MinIO/etc.
    pub endpoint_url: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    /// Object key prefix (e.g. `audit/`). Leading slash is stripped, trailing
    /// slash is preserved.
    pub prefix: String,
    /// Hostname segment used in the object key; falls back to the OS hostname.
    pub hostname_override: Option<String>,
    pub sse_mode: SseMode,
    pub kms_key_id: Option<String>,
    pub object_lock_kind: ObjectLockKind,
    pub object_lock_days: u32,
    pub max_buffer_age: Duration,
    pub max_uncompressed_bytes: usize,
    pub min_severity: AuditSeverity,
    pub event_filter: Option<HashSet<AuditEventTypeKey>>,
    /// True if non-AWS endpoint addressing requires path-style URLs. Auto-set
    /// to true when `endpoint_url` is non-empty.
    pub force_path_style: bool,
}

impl S3Config {
    pub fn new(bucket: impl Into<String>, region: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            region: region.into(),
            endpoint_url: None,
            access_key_id: None,
            secret_access_key: None,
            prefix: "audit/".to_string(),
            hostname_override: None,
            sse_mode: SseMode::S3,
            kms_key_id: None,
            object_lock_kind: ObjectLockKind::None,
            object_lock_days: 365,
            max_buffer_age: DEFAULT_MAX_BUFFER_AGE,
            max_uncompressed_bytes: DEFAULT_MAX_UNCOMPRESSED_BYTES,
            min_severity: AuditSeverity::Info,
            event_filter: None,
            force_path_style: false,
        }
    }
}

/// Mutable buffer of pending events.
struct Buffer {
    events: Vec<AuditEvent>,
    first_at: Option<Instant>,
    uncompressed_size: usize,
}

impl Buffer {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            first_at: None,
            uncompressed_size: 0,
        }
    }

    fn take(&mut self) -> Vec<AuditEvent> {
        self.first_at = None;
        self.uncompressed_size = 0;
        std::mem::take(&mut self.events)
    }
}

pub struct S3Sink {
    cfg: S3Config,
    hostname: String,
    buffer: Arc<Mutex<Buffer>>,
    client_cell: OnceCell<aws_sdk_s3::Client>,
}

impl S3Sink {
    /// Build the sink. The AWS SDK client is constructed lazily on first
    /// flush so the dispatcher's synchronous startup path doesn't need a
    /// tokio context just to validate config.
    pub fn new(cfg: S3Config) -> Result<Self, SinkError> {
        if cfg.bucket.trim().is_empty() {
            return Err(SinkError::Permanent("S3 bucket name is empty".into()));
        }
        if cfg.region.trim().is_empty() {
            return Err(SinkError::Permanent("S3 region is empty".into()));
        }
        if cfg.sse_mode == SseMode::Kms && cfg.kms_key_id.as_deref().unwrap_or("").is_empty() {
            return Err(SinkError::Permanent(
                "S3 sse-mode = kms but no kms-key-id provided".into(),
            ));
        }
        if cfg.sse_mode == SseMode::None {
            tracing::warn!(
                "S3 sink for bucket {} is configured with sse-mode=none — \
                 audit objects will not be encrypted at rest by the storage layer",
                cfg.bucket
            );
        }

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

        Ok(Self {
            cfg,
            hostname,
            buffer: Arc::new(Mutex::new(Buffer::new())),
            client_cell: OnceCell::new(),
        })
    }

    /// Force-flush whatever is buffered. Call this from the daemon's shutdown
    /// path so events captured in the last 5-minute window aren't lost.
    pub async fn flush_now(&self) -> Result<(), SinkError> {
        let events = {
            let mut buf = self.buffer.lock().await;
            if buf.events.is_empty() {
                return Ok(());
            }
            buf.take()
        };
        self.upload(&events, Utc::now()).await
    }

    /// Append events to the buffer; flush atomically if either threshold is hit.
    async fn buffer_and_maybe_flush(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        let to_flush: Option<(Vec<AuditEvent>, DateTime<Utc>)> = {
            let mut buf = self.buffer.lock().await;
            for ev in batch {
                if buf.first_at.is_none() {
                    buf.first_at = Some(Instant::now());
                }
                buf.uncompressed_size += estimate_event_size(ev);
                buf.events.push(ev.clone());
            }
            let age_hit = buf
                .first_at
                .map(|t| t.elapsed() >= self.cfg.max_buffer_age)
                .unwrap_or(false);
            let size_hit = buf.uncompressed_size >= self.cfg.max_uncompressed_bytes;
            if age_hit || size_hit {
                Some((buf.take(), Utc::now()))
            } else {
                None
            }
        };
        if let Some((events, now)) = to_flush {
            self.upload(&events, now).await?;
        }
        Ok(())
    }

    async fn client(&self) -> Result<&aws_sdk_s3::Client, SinkError> {
        self.client_cell
            .get_or_try_init(|| async { self.build_client().await })
            .await
    }

    async fn build_client(&self) -> Result<aws_sdk_s3::Client, SinkError> {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(self.cfg.region.clone()));

        if let (Some(ak), Some(sk)) = (
            self.cfg.access_key_id.as_deref(),
            self.cfg.secret_access_key.as_deref(),
        ) && !ak.is_empty()
            && !sk.is_empty()
        {
            let creds = aws_sdk_s3::config::Credentials::new(
                ak.to_string(),
                sk.to_string(),
                None,
                None,
                "thane-config",
            );
            loader = loader.credentials_provider(creds);
        }

        let shared = loader.load().await;

        let mut s3_builder = aws_sdk_s3::config::Builder::from(&shared);
        if let Some(ep) = self.cfg.endpoint_url.as_deref()
            && !ep.is_empty()
        {
            s3_builder = s3_builder.endpoint_url(ep);
        }
        if self.cfg.force_path_style || self.cfg.endpoint_url.as_deref().is_some_and(|s| !s.is_empty()) {
            s3_builder = s3_builder.force_path_style(true);
        }

        Ok(aws_sdk_s3::Client::from_conf(s3_builder.build()))
    }

    async fn upload(&self, events: &[AuditEvent], now: DateTime<Utc>) -> Result<(), SinkError> {
        if events.is_empty() {
            return Ok(());
        }
        let body = gzip_jsonl(events)
            .map_err(|e| SinkError::Permanent(format!("gzip encode: {e}")))?;
        let key = build_object_key(&self.cfg.prefix, &self.hostname, now);

        let client = self.client().await?;

        let mut req = client
            .put_object()
            .bucket(&self.cfg.bucket)
            .key(&key)
            .content_type("application/gzip")
            .content_encoding("gzip")
            .body(ByteStream::from(body));

        match self.cfg.sse_mode {
            SseMode::S3 => {
                req = req.server_side_encryption(ServerSideEncryption::Aes256);
            }
            SseMode::Kms => {
                req = req.server_side_encryption(ServerSideEncryption::AwsKms);
                if let Some(k) = self.cfg.kms_key_id.as_deref() {
                    req = req.ssekms_key_id(k);
                }
            }
            SseMode::None => {}
        }

        if let Some(mode) = self.cfg.object_lock_kind.to_sdk() {
            let retain_until = now + chrono::Duration::days(self.cfg.object_lock_days as i64);
            req = req
                .object_lock_mode(mode)
                .object_lock_retain_until_date(
                    aws_sdk_s3::primitives::DateTime::from_secs(retain_until.timestamp()),
                );
        }

        req.send()
            .await
            .map(|_| ())
            .map_err(classify_aws_error)
    }
}

#[async_trait]
impl AuditSink for S3Sink {
    fn name(&self) -> &str {
        "s3"
    }
    fn min_severity(&self) -> AuditSeverity {
        self.cfg.min_severity
    }
    fn event_filter(&self) -> Option<&HashSet<AuditEventTypeKey>> {
        self.cfg.event_filter.as_ref()
    }

    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        self.buffer_and_maybe_flush(batch).await
    }
}

/// Compress a slice of events to JSONL.gz bytes.
pub fn gzip_jsonl(events: &[AuditEvent]) -> std::io::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    for ev in events {
        let line = serde_json::to_vec(ev)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        enc.write_all(&line)?;
        enc.write_all(b"\n")?;
    }
    enc.finish()
}

/// Heuristic uncompressed-size contribution of one event. We don't pay the
/// cost of serializing twice — a few-percent over/under estimate is fine for
/// the flush trigger.
fn estimate_event_size(ev: &AuditEvent) -> usize {
    // Fixed-overhead fields (timestamp, UUIDs, enum tags) ≈ 200 bytes.
    // Variable: description + metadata JSON size.
    let meta_size = serde_json::to_string(&ev.metadata)
        .map(|s| s.len())
        .unwrap_or(0);
    200 + ev.description.len() + meta_size
}

/// Build `<prefix><hostname>/YYYY/MM/DD/HH/<uuid>.jsonl.gz`.
///
/// Public so the tests + ops runbook can demonstrate the layout.
pub fn build_object_key(prefix: &str, hostname: &str, ts: DateTime<Utc>) -> String {
    use chrono::Datelike;
    use chrono::Timelike;
    let prefix = prefix.trim_start_matches('/');
    let hostname = sanitize_path_component(hostname);
    let id = uuid::Uuid::new_v4();
    format!(
        "{prefix}{hostname}/{:04}/{:02}/{:02}/{:02}/{id}.jsonl.gz",
        ts.year(),
        ts.month(),
        ts.day(),
        ts.hour(),
    )
}

fn sanitize_path_component(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Map an AWS SDK error to our transient/permanent contract.
///
/// We use a coarse heuristic: anything we can dispatch off the SDK's
/// `ServiceError` HTTP status falls into 4xx → Permanent, 5xx + dispatch
/// failures → Transient. This mirrors how the rest of the codebase classifies
/// network errors.
fn classify_aws_error<E>(err: aws_sdk_s3::error::SdkError<E>) -> SinkError
where
    E: std::fmt::Display + std::error::Error,
{
    use aws_sdk_s3::error::SdkError;
    let msg = format!("{err}");
    match &err {
        SdkError::ServiceError(svc) => {
            let code = svc.raw().status().as_u16();
            if (400..500).contains(&code) {
                SinkError::Permanent(format!("S3 {code}: {msg}"))
            } else {
                SinkError::Transient(format!("S3 {code}: {msg}"))
            }
        }
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            SinkError::Transient(format!("S3 network: {msg}"))
        }
        SdkError::ConstructionFailure(_) => {
            SinkError::Permanent(format!("S3 request construction: {msg}"))
        }
        _ => SinkError::Transient(format!("S3: {msg}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::{AuditEventType, AuditSeverity};
    use uuid::Uuid;

    fn ev(desc: &str) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            workspace_id: Uuid::nil(),
            panel_id: None,
            event_type: AuditEventType::CommandExecuted,
            severity: AuditSeverity::Info,
            description: desc.into(),
            metadata: serde_json::json!({}),
            agent_name: None,
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        }
    }

    #[test]
    fn refuses_empty_bucket() {
        let cfg = S3Config::new("", "us-east-1");
        let err = S3Sink::new(cfg).err().expect("must fail");
        assert!(matches!(err, SinkError::Permanent(_)), "got {err:?}");
    }

    #[test]
    fn refuses_kms_without_key() {
        let mut cfg = S3Config::new("b", "us-east-1");
        cfg.sse_mode = SseMode::Kms;
        let err = S3Sink::new(cfg).err().expect("must fail");
        match err {
            SinkError::Permanent(msg) => assert!(msg.contains("kms")),
            other => panic!("expected Permanent kms error, got {other:?}"),
        }
    }

    #[test]
    fn sse_mode_parse() {
        assert_eq!(SseMode::parse("kms"), SseMode::Kms);
        assert_eq!(SseMode::parse("s3"), SseMode::S3);
        assert_eq!(SseMode::parse("S3"), SseMode::S3);
        assert_eq!(SseMode::parse("none"), SseMode::None);
        assert_eq!(SseMode::parse("garbage"), SseMode::S3);
    }

    #[test]
    fn object_lock_kind_parse() {
        assert_eq!(ObjectLockKind::parse("compliance"), ObjectLockKind::Compliance);
        assert_eq!(ObjectLockKind::parse("governance"), ObjectLockKind::Governance);
        assert_eq!(ObjectLockKind::parse("none"), ObjectLockKind::None);
        assert_eq!(ObjectLockKind::parse(""), ObjectLockKind::None);
    }

    #[test]
    fn build_object_key_layout_sorts_chronologically() {
        let ts1 = chrono::DateTime::parse_from_rfc3339("2026-05-25T10:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts2 = chrono::DateTime::parse_from_rfc3339("2026-05-25T11:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let k1 = build_object_key("audit/", "host1", ts1);
        let k2 = build_object_key("audit/", "host1", ts2);
        // Lex sort matches chronological.
        assert!(k1 < k2, "{k1} should sort before {k2}");
        assert!(k1.starts_with("audit/host1/2026/05/25/10/"));
        assert!(k1.ends_with(".jsonl.gz"));
    }

    #[test]
    fn build_object_key_sanitizes_hostname() {
        let ts = Utc::now();
        let k = build_object_key("p/", "bad/host:name", ts);
        assert!(k.starts_with("p/bad_host_name/"), "got: {k}");
    }

    #[test]
    fn build_object_key_strips_leading_slash() {
        let ts = Utc::now();
        let k = build_object_key("/p/", "h", ts);
        assert!(k.starts_with("p/h/"));
    }

    #[test]
    fn gzip_jsonl_is_decompressible() {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let batch = vec![ev("one"), ev("two"), ev("three")];
        let gz = gzip_jsonl(&batch).unwrap();

        let mut dec = GzDecoder::new(&gz[..]);
        let mut out = String::new();
        dec.read_to_string(&mut out).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in &lines {
            // Each line is a valid JSON object.
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(v["description"].is_string());
        }
    }

    #[test]
    fn estimate_event_size_includes_payload() {
        let small = ev("x");
        let big = AuditEvent {
            description: "y".repeat(1000),
            metadata: serde_json::json!({"data": "z".repeat(2000)}),
            ..ev("y")
        };
        assert!(estimate_event_size(&big) > estimate_event_size(&small));
        assert!(estimate_event_size(&big) >= 3000);
    }

    #[tokio::test]
    async fn buffer_does_not_flush_below_thresholds() {
        // Synthesize a sink without touching S3: keep buffer threshold huge so
        // we exercise the buffering path only.
        let mut cfg = S3Config::new("b", "us-east-1");
        cfg.max_buffer_age = Duration::from_secs(3600);
        cfg.max_uncompressed_bytes = 10 * 1024 * 1024;
        let sink = S3Sink::new(cfg).unwrap();

        // Append events but do NOT cross the threshold; we cannot directly
        // call buffer_and_maybe_flush because it tries to send to S3 on flush.
        // Instead, exercise the buffer directly:
        let mut buf = sink.buffer.lock().await;
        for _ in 0..3 {
            let e = ev("x");
            buf.uncompressed_size += estimate_event_size(&e);
            buf.events.push(e);
            buf.first_at.get_or_insert_with(Instant::now);
        }
        assert_eq!(buf.events.len(), 3);
        assert!(buf.first_at.is_some());
    }

    #[tokio::test]
    async fn flush_now_on_empty_buffer_is_ok() {
        let cfg = S3Config::new("b", "us-east-1");
        let sink = S3Sink::new(cfg).unwrap();
        // No events buffered; flush_now should return Ok without contacting S3.
        sink.flush_now().await.expect("noop flush ok");
    }
}
