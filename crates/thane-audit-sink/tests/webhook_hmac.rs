//! End-to-end webhook HMAC test: spin up a tiny TCP server, point the sink at
//! it, read the request, verify `X-Thane-Signature` decodes correctly.

#![cfg(feature = "webhook")]

use std::time::Duration;

use thane_audit_sink::webhook::{WebhookConfig, WebhookSink, parse_signature_header, sign_payload};
use thane_audit_sink::AuditSink;
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use uuid::Uuid;

fn ev() -> AuditEvent {
    AuditEvent {
        id: Uuid::nil(),
        timestamp: chrono::Utc::now(),
        workspace_id: Uuid::nil(),
        panel_id: None,
        event_type: AuditEventType::CommandExecuted,
        severity: AuditSeverity::Info,
        description: "x".into(),
        metadata: serde_json::json!({}),
        agent_name: None,
        system_user: None,
        system_uid: None,
        prev_hash: String::new(),
        hmac: None,
    }
}

/// Read an HTTP request from a TCP stream. Returns (headers map, body bytes).
async fn read_request(stream: &mut tokio::net::TcpStream)
    -> (std::collections::HashMap<String, String>, Vec<u8>)
{
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).await.unwrap();
        if n == 0 { break; }
        buf.extend_from_slice(&tmp[..n]);
        // Stop once we've seen full headers + the declared body.
        if let Some(hdr_end) = find_header_end(&buf) {
            let (hdrs, _body) = parse_headers(&buf[..hdr_end]);
            let body_len: usize = hdrs.get("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if buf.len() >= hdr_end + 4 + body_len { break; }
        }
    }
    let hdr_end = find_header_end(&buf).expect("end of headers");
    let (hdrs, _) = parse_headers(&buf[..hdr_end]);
    let body = buf[hdr_end + 4..].to_vec();
    (hdrs, body)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_headers(buf: &[u8]) -> (std::collections::HashMap<String, String>, ()) {
    let mut hdrs = std::collections::HashMap::new();
    let s = std::str::from_utf8(buf).unwrap_or("");
    for line in s.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            hdrs.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    (hdrs, ())
}

#[tokio::test(flavor = "current_thread")]
async fn webhook_request_carries_valid_hmac() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}/ingest", addr);

    let secret = b"super-secret".to_vec();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let result = timeout(Duration::from_secs(2), read_request(&mut sock)).await.unwrap();
        // Send a 200 so the sink reports success.
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        result
    });

    let mut cfg = WebhookConfig::new(url, secret.clone());
    cfg.hostname_override = Some("test-host".into());
    let sink = WebhookSink::new(cfg).unwrap();

    sink.send(&[ev()]).await.expect("delivery");

    let (hdrs, body) = server.await.unwrap();
    assert_eq!(hdrs.get("content-type").map(|s| s.as_str()), Some("application/json"));
    let sig_header = hdrs.get("x-thane-signature").expect("signature header present");
    let (ts, sig) = parse_signature_header(sig_header).expect("parseable signature");
    let expected = sign_payload(&secret, ts, &body);
    assert_eq!(sig, expected, "X-Thane-Signature must HMAC over t.body with the configured secret");

    // Body must declare the schema_version, host, and at least one event.
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["schema_version"], "1");
    assert_eq!(parsed["host"], "test-host");
    assert!(parsed["batch"].as_array().unwrap().len() == 1);
}
