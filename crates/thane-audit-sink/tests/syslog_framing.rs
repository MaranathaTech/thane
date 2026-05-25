//! End-to-end test of RFC 5424 + RFC 6587 octet-counting framing.
//!
//! Sketches a TCP listener, has the SyslogSink connect + deliver one batch,
//! reads the raw bytes from the wire, and asserts both the frame envelope
//! and the syslog header parse cleanly.

#![cfg(feature = "syslog")]

use std::time::Duration;

use thane_audit_sink::syslog::{SyslogConfig, SyslogSink};
use thane_audit_sink::AuditSink;
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::time::timeout;
use uuid::Uuid;

fn ev() -> AuditEvent {
    AuditEvent {
        id: Uuid::nil(),
        timestamp: chrono::DateTime::parse_from_rfc3339("2026-05-25T10:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        workspace_id: Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap(),
        panel_id: None,
        event_type: AuditEventType::SecretAccess,
        severity: AuditSeverity::Alert,
        description: "leaked .env".into(),
        metadata: serde_json::json!({"path": "/tmp/.env"}),
        agent_name: Some("claude".into()),
        system_user: None,
        system_uid: None,
        prev_hash: String::new(),
        hmac: None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn syslog_sink_writes_octet_counted_rfc5424_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn the server side: read until the connection closes.
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        // Read a chunk; the sink writes synchronously per event then flushes.
        let mut tmp = [0u8; 4096];
        // 1s is plenty since the sink writes immediately.
        let _ = timeout(Duration::from_secs(2), async {
            loop {
                match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        // Stop once we have a complete frame.
                        if buf.contains(&b' ')
                            && let Some(sp) = buf.iter().position(|&b| b == b' ')
                            && let Ok(len_s) = std::str::from_utf8(&buf[..sp])
                            && let Ok(n_expected) = len_s.parse::<usize>()
                            && buf.len() >= sp + 1 + n_expected
                        {
                            break;
                        }
                    }
                }
            }
        })
        .await;
        buf
    });

    let mut cfg = SyslogConfig::new(addr.ip().to_string(), addr.port());
    cfg.use_tls = false;
    cfg.hostname_override = Some("test-host".to_string());
    cfg.app_name = "thane".to_string();
    let sink = SyslogSink::new(cfg).expect("sink");

    sink.send(&[ev()]).await.expect("delivery");
    // Drop the sink so its connection closes and the server task ends.
    drop(sink);

    let bytes = server.await.unwrap();
    assert!(!bytes.is_empty(), "server should have read data");

    // Parse the octet-counted frame: "<n> <message>".
    let sp = bytes.iter().position(|&b| b == b' ').unwrap();
    let count: usize = std::str::from_utf8(&bytes[..sp]).unwrap().parse().unwrap();
    let msg_start = sp + 1;
    let msg_end = msg_start + count;
    assert!(bytes.len() >= msg_end, "frame must contain {count} byte message");
    let msg = std::str::from_utf8(&bytes[msg_start..msg_end]).unwrap();

    // PRI = facility(13)*8 + severity(alert=1) = 105.
    assert!(msg.starts_with("<105>1 2026-05-25T10:30:00+00:00 test-host thane "), "got: {msg}");
    assert!(msg.contains("secret_access"));
    assert!(msg.contains("[thane@32473 workspace=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\""));
    assert!(msg.contains("agent=\"claude\""));
    // JSON payload still present at the end.
    assert!(msg.contains("\"description\":\"leaked .env\""));
}
