//! S3 sink integration tests that don't need a live S3 endpoint.
//!
//! These cover the pieces the AWS SDK doesn't help us verify: object key
//! layout, gzip framing, and the severity-mapping plumbing through the trait.
//! Live S3 / MinIO smoke tests are documented as `--ignored` runs in
//! `AUDIT_LOG.md`.

#![cfg(feature = "s3")]

use flate2::read::GzDecoder;
use std::io::Read;

use thane_audit_sink::s3::{
    ObjectLockKind, S3Config, S3Sink, SseMode, build_object_key, gzip_jsonl,
};
use thane_audit_sink::SinkError;
use thane_core::audit::{AuditEvent, AuditEventType, AuditSeverity};
use uuid::Uuid;

fn ev(sev: AuditSeverity, desc: &str) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        workspace_id: Uuid::nil(),
        panel_id: None,
        event_type: AuditEventType::CommandExecuted,
        severity: sev,
        description: desc.into(),
        metadata: serde_json::json!({"k": "v"}),
        agent_name: None,
        system_user: None,
        system_uid: None,
        prev_hash: String::new(),
        hmac: None,
    }
}

#[test]
fn object_key_hierarchical_and_sorted() {
    let t1 = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let t2 = chrono::DateTime::parse_from_rfc3339("2026-01-02T04:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let k1 = build_object_key("audit/", "host-1", t1);
    let k2 = build_object_key("audit/", "host-1", t2);
    assert!(k1.starts_with("audit/host-1/2026/01/02/03/"));
    assert!(k2.starts_with("audit/host-1/2026/01/02/04/"));
    assert!(k1 < k2);
    assert!(k1.ends_with(".jsonl.gz"));
}

#[test]
fn gzip_jsonl_round_trip() {
    let events = vec![
        ev(AuditSeverity::Info, "alpha"),
        ev(AuditSeverity::Alert, "bravo"),
    ];
    let gz = gzip_jsonl(&events).expect("encode");
    let mut dec = GzDecoder::new(&gz[..]);
    let mut s = String::new();
    dec.read_to_string(&mut s).expect("decode");
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 2);
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["description"], "alpha");
    let v: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(v["description"], "bravo");
}

#[test]
fn refuses_kms_mode_without_kms_key() {
    let mut cfg = S3Config::new("b", "us-east-1");
    cfg.sse_mode = SseMode::Kms;
    let err = S3Sink::new(cfg).err().expect("must fail");
    match err {
        SinkError::Permanent(msg) => assert!(msg.to_lowercase().contains("kms")),
        other => panic!("expected Permanent error, got {other:?}"),
    }
}

#[test]
fn accepts_kms_mode_with_kms_key() {
    let mut cfg = S3Config::new("b", "us-east-1");
    cfg.sse_mode = SseMode::Kms;
    cfg.kms_key_id = Some("alias/my-key".into());
    assert!(S3Sink::new(cfg).is_ok());
}

#[test]
fn parses_sse_and_object_lock_strings() {
    assert_eq!(SseMode::parse("s3"), SseMode::S3);
    assert_eq!(SseMode::parse("kms"), SseMode::Kms);
    assert_eq!(SseMode::parse("none"), SseMode::None);
    assert_eq!(SseMode::parse("garbage"), SseMode::S3);

    assert_eq!(
        ObjectLockKind::parse("compliance"),
        ObjectLockKind::Compliance
    );
    assert_eq!(
        ObjectLockKind::parse("governance"),
        ObjectLockKind::Governance
    );
    assert_eq!(ObjectLockKind::parse(""), ObjectLockKind::None);
}
