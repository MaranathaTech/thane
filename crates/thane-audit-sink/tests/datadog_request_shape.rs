//! End-to-end Datadog Logs test: small TCP server pretends to be the v2
//! logs intake. Verifies request headers + JSON array body shape.

#![cfg(feature = "datadog")]

use std::collections::HashMap;
use std::time::Duration;

use thane_audit_sink::AuditSink;
use thane_audit_sink::datadog::{
    DatadogConfig, DatadogRegion, DatadogSink, severity_to_status,
};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use uuid::Uuid;

fn ev(sev: AuditSeverity, ty: AuditEventType, desc: &str) -> AuditEvent {
    AuditEvent {
        id: Uuid::nil(),
        timestamp: chrono::Utc::now(),
        workspace_id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
        panel_id: None,
        event_type: ty,
        severity: sev,
        description: desc.into(),
        metadata: serde_json::json!({"k": "v"}),
        agent_name: Some("claude".into()),
        system_user: None,
        system_uid: None,
        prev_hash: String::new(),
        hmac: None,
    }
}

async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
) -> (HashMap<String, String>, Vec<u8>) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(end) = find_header_end(&buf) {
            let hdrs = parse_headers(&buf[..end]);
            let cl: usize = hdrs
                .get("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + cl {
                break;
            }
        }
    }
    let end = find_header_end(&buf).expect("end of headers");
    let hdrs = parse_headers(&buf[..end]);
    let body = buf[end + 4..].to_vec();
    (hdrs, body)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_headers(buf: &[u8]) -> HashMap<String, String> {
    let mut hdrs = HashMap::new();
    let s = std::str::from_utf8(buf).unwrap_or("");
    for line in s.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            hdrs.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    hdrs
}

#[tokio::test(flavor = "current_thread")]
async fn datadog_request_has_api_key_header_and_array_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}/api/v2/logs", addr);

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let result = timeout(Duration::from_secs(2), read_http_request(&mut sock))
            .await
            .unwrap();
        sock.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        result
    });

    let mut cfg = DatadogConfig::new(DatadogRegion::Us1, "my-dd-key");
    cfg.hostname_override = Some("test-host".into());
    cfg.env = "staging".into();
    cfg.url_override = Some(url);
    let sink = DatadogSink::new(cfg).unwrap();

    sink.send(&[
        ev(AuditSeverity::Info, AuditEventType::CommandExecuted, "a"),
        ev(AuditSeverity::Alert, AuditEventType::SecretAccess, "b"),
    ])
    .await
    .expect("delivery");

    let (hdrs, body) = server.await.unwrap();

    assert_eq!(hdrs.get("dd-api-key").map(|s| s.as_str()), Some("my-dd-key"));
    assert_eq!(
        hdrs.get("content-type").map(|s| s.as_str()),
        Some("application/json")
    );

    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = parsed.as_array().expect("body must be an array");
    assert_eq!(arr.len(), 2);
    for entry in arr {
        assert_eq!(entry["hostname"], "test-host");
        assert_eq!(entry["service"], "thane");
        assert_eq!(entry["ddsource"], "thane");
        assert!(entry["thane"].is_object(), "nested envelope present");
        assert!(entry["ddtags"]
            .as_str()
            .unwrap()
            .contains("env:staging"));
    }
    // Severity mapping reaches the wire.
    assert_eq!(arr[0]["status"], severity_to_status(AuditSeverity::Info));
    assert_eq!(arr[1]["status"], severity_to_status(AuditSeverity::Alert));
}

#[tokio::test(flavor = "current_thread")]
async fn datadog_429_is_transient_403_is_permanent() {
    use thane_audit_sink::SinkError;

    // 429 → Transient (per-org rate limit)
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}/api/v2/logs", addr);

        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _ = timeout(Duration::from_secs(2), read_http_request(&mut sock)).await;
            sock.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let mut cfg = DatadogConfig::new(DatadogRegion::Us1, "k");
        cfg.url_override = Some(url);
        let sink = DatadogSink::new(cfg).unwrap();
        let err = sink
            .send(&[ev(AuditSeverity::Info, AuditEventType::CommandExecuted, "x")])
            .await
            .expect_err("expected error");
        assert!(matches!(err, SinkError::Transient(_)), "got: {err:?}");
    }

    // 403 → Permanent (bad API key)
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}/api/v2/logs", addr);

        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _ = timeout(Duration::from_secs(2), read_http_request(&mut sock)).await;
            sock.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let mut cfg = DatadogConfig::new(DatadogRegion::Us1, "k");
        cfg.url_override = Some(url);
        let sink = DatadogSink::new(cfg).unwrap();
        let err = sink
            .send(&[ev(AuditSeverity::Info, AuditEventType::CommandExecuted, "x")])
            .await
            .expect_err("expected error");
        assert!(matches!(err, SinkError::Permanent(_)), "got: {err:?}");
    }
}
