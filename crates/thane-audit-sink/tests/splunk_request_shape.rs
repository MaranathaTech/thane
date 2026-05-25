//! End-to-end Splunk HEC test: spin up a TCP server pretending to be a Splunk
//! HEC endpoint, confirm the request headers + NDJSON body shape match what
//! Splunk expects.

#![cfg(feature = "splunk")]

use std::collections::HashMap;
use std::time::Duration;

use thane_audit_sink::AuditSink;
use thane_audit_sink::splunk_hec::{SplunkHecConfig, SplunkHecSink, parse_ndjson_body};
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use uuid::Uuid;

fn ev(sev: AuditSeverity, ty: AuditEventType, desc: &str) -> AuditEvent {
    AuditEvent {
        id: Uuid::nil(),
        timestamp: chrono::DateTime::parse_from_rfc3339("2026-05-25T10:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        workspace_id: Uuid::nil(),
        panel_id: None,
        event_type: ty,
        severity: sev,
        description: desc.into(),
        metadata: serde_json::json!({}),
        agent_name: None,
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
async fn splunk_request_carries_auth_header_and_ndjson_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}/services/collector/event", addr);

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let result = timeout(Duration::from_secs(2), read_http_request(&mut sock))
            .await
            .unwrap();
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 27\r\n\r\n{\"text\":\"Success\",\"code\":0}")
            .await
            .unwrap();
        result
    });

    let mut cfg = SplunkHecConfig::new(url, "my-hec-token");
    cfg.hostname_override = Some("test-host".into());
    cfg.index = Some("audit".into());
    let sink = SplunkHecSink::new(cfg).unwrap();

    sink.send(&[
        ev(AuditSeverity::Info, AuditEventType::CommandExecuted, "first"),
        ev(AuditSeverity::Alert, AuditEventType::SecretAccess, "second"),
    ])
    .await
    .expect("delivery");

    let (hdrs, body) = server.await.unwrap();

    assert_eq!(
        hdrs.get("authorization").map(|s| s.as_str()),
        Some("Splunk my-hec-token"),
        "Authorization header must be `Splunk <token>`"
    );
    assert_eq!(
        hdrs.get("content-type").map(|s| s.as_str()),
        Some("application/json")
    );

    let envs = parse_ndjson_body(&body);
    assert_eq!(envs.len(), 2);
    assert_eq!(envs[0]["host"], "test-host");
    assert_eq!(envs[0]["index"], "audit");
    assert_eq!(envs[0]["sourcetype"], "thane:audit");
    assert_eq!(envs[0]["event"]["description"], "first");
    assert_eq!(envs[1]["event"]["description"], "second");
}

#[tokio::test(flavor = "current_thread")]
async fn splunk_5xx_is_transient_4xx_is_permanent() {
    use thane_audit_sink::SinkError;

    // 500 → Transient
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}/services/collector/event", addr);

        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _ = timeout(Duration::from_secs(2), read_http_request(&mut sock)).await;
            sock.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 5\r\n\r\noh no")
                .await
                .unwrap();
        });

        let cfg = SplunkHecConfig::new(url, "t");
        let sink = SplunkHecSink::new(cfg).unwrap();
        let err = sink
            .send(&[ev(AuditSeverity::Info, AuditEventType::CommandExecuted, "x")])
            .await
            .expect_err("expected error");
        assert!(matches!(err, SinkError::Transient(_)), "got: {err:?}");
    }

    // 403 → Permanent
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}/services/collector/event", addr);

        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let _ = timeout(Duration::from_secs(2), read_http_request(&mut sock)).await;
            sock.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 11\r\n\r\nbad token!!")
                .await
                .unwrap();
        });

        let cfg = SplunkHecConfig::new(url, "t");
        let sink = SplunkHecSink::new(cfg).unwrap();
        let err = sink
            .send(&[ev(AuditSeverity::Info, AuditEventType::CommandExecuted, "x")])
            .await
            .expect_err("expected error");
        assert!(matches!(err, SinkError::Permanent(_)), "got: {err:?}");
    }
}
