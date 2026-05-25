//! RFC 5424 syslog sink over TCP, with optional TLS and RFC 6587 octet
//! counting framing.
//!
//! Wire format per message:
//!
//! ```text
//! <PRI>1 <TIMESTAMP> <HOSTNAME> <APP> <PROCID> <MSGID>
//!   [thane@32473 workspace="..." agent="..." severity="..."] <JSON_PAYLOAD>
//! ```
//!
//! `PRI = facility*8 + severity` per RFC 5424 §6.2.1. We use facility 13
//! ("log audit"). Severity numerics:
//!   info=6, warning=4, alert=1, critical=2.
//!
//! Octet-counted framing per RFC 6587 §3.4.1:
//!
//! ```text
//! <length> <SP> <message>
//! ```
//!
//! Connection is reused across batches and re-established on I/O failure with
//! the dispatcher's backoff (we only return a `Transient` error and let the
//! retry layer pace reconnect attempts).

use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, RootCertStore};
use rustls_pemfile::certs;
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::{AuditEventTypeKey, AuditSink, SinkError};

const SYSLOG_FACILITY_LOG_AUDIT: u8 = 13;
/// Private Enterprise Number used in the SD-ID. 32473 is the IANA-reserved
/// example number for documentation; replace with the real thane PEN once
/// allocated.
const SD_ID: &str = "thane@32473";

/// Configuration for [`SyslogSink`].
#[derive(Debug, Clone)]
pub struct SyslogConfig {
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    /// Optional path to an extra trusted CA certificate (PEM). Concatenated
    /// with the system trust store.
    pub ca_cert_path: Option<std::path::PathBuf>,
    /// The APP-NAME field in the syslog header.
    pub app_name: String,
    pub min_severity: AuditSeverity,
    pub event_filter: Option<HashSet<AuditEventTypeKey>>,
    /// Override hostname (otherwise the OS hostname is used at construction).
    pub hostname_override: Option<String>,
}

impl SyslogConfig {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            use_tls: true,
            ca_cert_path: None,
            app_name: "thane".to_string(),
            min_severity: AuditSeverity::Info,
            event_filter: None,
            hostname_override: None,
        }
    }
}

enum Connection {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl Connection {
    async fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            Connection::Plain(s) => s.write_all(buf).await,
            Connection::Tls(s) => s.write_all(buf).await,
        }
    }
    async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Connection::Plain(s) => s.flush().await,
            Connection::Tls(s) => s.flush().await,
        }
    }
}

/// Syslog sink over TCP (+ optional TLS).
pub struct SyslogSink {
    cfg: SyslogConfig,
    hostname: String,
    procid: String,
    /// Lazy connection, reused across batches. Held behind a tokio Mutex so
    /// only one batch is on the wire at a time.
    conn: Mutex<Option<Connection>>,
    /// Pre-built TLS connector, reused. None when use_tls = false.
    tls_connector: Option<TlsConnector>,
}

impl SyslogSink {
    /// Build a sink from config. Fails synchronously only if TLS setup is
    /// requested but the trust store cannot be assembled.
    pub fn new(cfg: SyslogConfig) -> Result<Self, std::io::Error> {
        let hostname = cfg
            .hostname_override
            .clone()
            .unwrap_or_else(|| {
                let h = whoami::fallible::hostname().unwrap_or_default();
                if h.is_empty() { "thane".to_string() } else { h }
            });
        let procid = std::process::id().to_string();

        let tls_connector = if cfg.use_tls {
            Some(build_tls_connector(cfg.ca_cert_path.as_deref())?)
        } else {
            None
        };

        Ok(Self {
            cfg,
            hostname,
            procid,
            conn: Mutex::new(None),
            tls_connector,
        })
    }

    /// Build (or rebuild) the TCP / TLS connection.
    async fn connect(&self) -> Result<Connection, SinkError> {
        let addr = format!("{}:{}", self.cfg.host, self.cfg.port);
        let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&addr))
            .await
            .map_err(|_| SinkError::Transient(format!("connect timeout to {addr}")))?
            .map_err(|e| SinkError::Transient(format!("connect to {addr}: {e}")))?;

        if let Some(connector) = &self.tls_connector {
            let server_name = ServerName::try_from(self.cfg.host.clone())
                .map_err(|e| SinkError::Permanent(format!("invalid TLS server name: {e}")))?;
            let tls = connector
                .connect(server_name, tcp)
                .await
                .map_err(|e| SinkError::Transient(format!("TLS handshake: {e}")))?;
            Ok(Connection::Tls(Box::new(tls)))
        } else {
            Ok(Connection::Plain(tcp))
        }
    }

    /// Render one audit event as a framed RFC 5424 message ready to write.
    pub fn frame_event(&self, event: &AuditEvent) -> Vec<u8> {
        let msg = format_syslog_message(event, &self.hostname, &self.cfg.app_name, &self.procid);
        frame_with_octet_count(msg.as_bytes())
    }
}

#[async_trait]
impl AuditSink for SyslogSink {
    fn name(&self) -> &str { "syslog" }
    fn min_severity(&self) -> AuditSeverity { self.cfg.min_severity }
    fn event_filter(&self) -> Option<&HashSet<AuditEventTypeKey>> {
        self.cfg.event_filter.as_ref()
    }

    async fn send(&self, batch: &[AuditEvent]) -> Result<(), SinkError> {
        let mut guard = self.conn.lock().await;

        for event in batch {
            let bytes = self.frame_event(event);

            // (Re)connect lazily.
            if guard.is_none() {
                *guard = Some(self.connect().await?);
            }

            // Try to write; on I/O error, reconnect once and retry the event.
            let mut conn = guard.take().unwrap();
            if let Err(e) = conn.write_all(&bytes).await {
                tracing::warn!("syslog write failed, reconnecting: {e}");
                // Drop the broken connection; force fresh one next iteration.
                let new = self.connect().await?;
                *guard = Some(new);
                let mut conn = guard.take().unwrap();
                conn.write_all(&bytes)
                    .await
                    .map_err(|e| SinkError::Transient(format!("write after reconnect: {e}")))?;
                conn.flush()
                    .await
                    .map_err(|e| SinkError::Transient(format!("flush after reconnect: {e}")))?;
                *guard = Some(conn);
            } else {
                *guard = Some(conn);
            }
        }

        if let Some(conn) = guard.as_mut() {
            conn.flush()
                .await
                .map_err(|e| SinkError::Transient(format!("final flush: {e}")))?;
        }

        Ok(())
    }
}

/// Wrap `msg` in RFC 6587 octet-counted framing: `<len> <SP> <msg>`.
pub fn frame_with_octet_count(msg: &[u8]) -> Vec<u8> {
    let len = msg.len();
    let header = format!("{len} ");
    let mut out = Vec::with_capacity(header.len() + msg.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(msg);
    out
}

/// Build the full RFC 5424 message string (no framing).
///
/// Public so the test harness and the Webhook sink's "preview" path can reuse it.
pub fn format_syslog_message(
    event: &AuditEvent,
    hostname: &str,
    app_name: &str,
    procid: &str,
) -> String {
    let pri = compute_pri(event.severity);
    let timestamp = event.timestamp.to_rfc3339();
    let msgid = msgid_for(&event.event_type);
    let sd = structured_data(event);
    let payload = serde_json::to_string(event)
        .unwrap_or_else(|_| String::from("\"<serialize_failed>\""));
    format!("<{pri}>1 {timestamp} {hostname} {app_name} {procid} {msgid} {sd} {payload}")
}

fn compute_pri(severity: AuditSeverity) -> u8 {
    let sev_num: u8 = match severity {
        AuditSeverity::Info => 6,
        AuditSeverity::Warning => 4,
        AuditSeverity::Alert => 1,
        AuditSeverity::Critical => 2,
    };
    SYSLOG_FACILITY_LOG_AUDIT * 8 + sev_num
}

fn msgid_for(t: &AuditEventType) -> String {
    crate::event_type_key(t)
}

/// Build the SD-1 element. Always emits the `thane@32473` SD-ID with
/// workspace, severity, and optionally agent name fields.
fn structured_data(event: &AuditEvent) -> String {
    let mut params = vec![
        format!("workspace=\"{}\"", event.workspace_id),
        format!("severity=\"{}\"", severity_label(event.severity)),
    ];
    if let Some(agent) = &event.agent_name {
        params.push(format!("agent=\"{}\"", sd_param_escape(agent)));
    }
    if let Some(user) = &event.system_user {
        params.push(format!("user=\"{}\"", sd_param_escape(user)));
    }
    format!("[{SD_ID} {}]", params.join(" "))
}

fn severity_label(s: AuditSeverity) -> &'static str {
    match s {
        AuditSeverity::Info => "info",
        AuditSeverity::Warning => "warning",
        AuditSeverity::Alert => "alert",
        AuditSeverity::Critical => "critical",
    }
}

/// Escape an SD-PARAM-VALUE per RFC 5424 §6.3.3: `"`, `\`, `]` get a
/// backslash. We never see UTF-8 control characters in our fields, so this is
/// enough.
fn sd_param_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '\\' | ']' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Build a rustls `TlsConnector` using the system trust store, optionally
/// augmented with a user-provided PEM CA file.
fn build_tls_connector(ca_cert_path: Option<&Path>) -> Result<TlsConnector, std::io::Error> {
    // Ensure the default crypto provider is installed once for this process.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut roots = RootCertStore::empty();

    // Try system roots first; if unavailable, fall back to webpki-roots.
    match rustls_native_certs::load_native_certs() {
        result if !result.certs.is_empty() => {
            for cert in result.certs {
                if let Err(e) = roots.add(cert) {
                    tracing::warn!("ignoring bad system cert: {e}");
                }
            }
        }
        result => {
            tracing::info!(
                "no system trust store ({} errors); falling back to webpki-roots",
                result.errors.len()
            );
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
    }

    if let Some(path) = ca_cert_path {
        let pem = std::fs::read(path)?;
        let mut cur = Cursor::new(pem);
        for cert_result in certs(&mut cur) {
            let cert: CertificateDer<'static> = cert_result?;
            roots
                .add(cert)
                .map_err(|e| std::io::Error::other(format!("bad CA cert: {e}")))?;
        }
    }

    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn fake_event(sev: AuditSeverity, ty: AuditEventType) -> AuditEvent {
        AuditEvent {
            id: Uuid::nil(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-05-25T10:30:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            workspace_id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            panel_id: None,
            event_type: ty,
            severity: sev,
            description: "ran cmd".to_string(),
            metadata: serde_json::json!({"command": "ls"}),
            agent_name: Some("claude".to_string()),
            system_user: Some("alice".to_string()),
            system_uid: Some(501),
            prev_hash: String::new(),
            hmac: None,
        }
    }

    #[test]
    fn pri_matches_rfc_5424_facility_severity_math() {
        // facility 13 * 8 = 104, +6 (info) = 110
        assert_eq!(compute_pri(AuditSeverity::Info), 110);
        // +4 (warning) = 108
        assert_eq!(compute_pri(AuditSeverity::Warning), 108);
        assert_eq!(compute_pri(AuditSeverity::Alert), 105);
        assert_eq!(compute_pri(AuditSeverity::Critical), 106);
    }

    #[test]
    fn format_produces_rfc5424_skeleton() {
        let ev = fake_event(AuditSeverity::Info, AuditEventType::CommandExecuted);
        let msg = format_syslog_message(&ev, "host1", "thane", "9001");
        assert!(msg.starts_with("<110>1 2026-05-25T10:30:00+00:00 host1 thane 9001 command_executed "));
        assert!(msg.contains("[thane@32473 workspace=\"11111111-1111-1111-1111-111111111111\""));
        assert!(msg.contains("agent=\"claude\""));
        assert!(msg.contains("user=\"alice\""));
    }

    #[test]
    fn frame_prepends_octet_count() {
        let body = b"hello";
        let framed = frame_with_octet_count(body);
        assert_eq!(framed, b"5 hello".to_vec());
    }

    #[test]
    fn full_frame_octet_count_matches_message_length() {
        let ev = fake_event(AuditSeverity::Alert, AuditEventType::SecretAccess);
        let msg = format_syslog_message(&ev, "h", "thane", "1");
        let framed = frame_with_octet_count(msg.as_bytes());

        let space_pos = framed.iter().position(|&b| b == b' ').unwrap();
        let count: usize = std::str::from_utf8(&framed[..space_pos])
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(count, framed.len() - space_pos - 1);
        assert_eq!(&framed[space_pos + 1..], msg.as_bytes());
    }

    #[test]
    fn sd_param_escapes_special_chars() {
        assert_eq!(sd_param_escape("ab\"c]\\d"), "ab\\\"c\\]\\\\d");
        assert_eq!(sd_param_escape("plain"), "plain");
    }

    #[test]
    fn msgid_uses_snake_case_event_type() {
        assert_eq!(msgid_for(&AuditEventType::CommandExecuted), "command_executed");
        assert_eq!(msgid_for(&AuditEventType::QueueTaskSubmitted), "queue_task_submitted");
    }
}
