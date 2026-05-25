//! Audit-log redaction (Phase 3 of the audit hardening).
//!
//! Detected PII and secrets are scrubbed BEFORE the HMAC is computed and
//! BEFORE the JSONL line is written, so the on-disk record never contains
//! the raw secret. The HMAC is computed over the redacted form — running
//! `verify_integrity` after the fact confirms exactly what is on disk.
//!
//! Patterns are compiled once at first use via `LazyLock` to keep hot-path
//! cost minimal.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit::{extract_file_paths, is_sensitive_file, AuditEvent, AuditEventType};

/// How aggressively to redact audit events before they hit disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RedactionPolicy {
    /// Store events verbatim. Dev only — no compliance value.
    None,
    /// Scrub detected secrets and PII; structure preserved.
    #[default]
    Redact,
    /// `Redact` + replace `description` with a fixed event-type string and
    /// drop `metadata` entirely. Keeps only the structural skeleton.
    Strict,
}

impl RedactionPolicy {
    /// Parse a string from the config file. Unknown values fall back to `Redact`
    /// so a typo can't accidentally disable redaction.
    pub fn from_config_value(v: &str) -> Self {
        match v.trim().to_ascii_lowercase().as_str() {
            "none" => RedactionPolicy::None,
            "strict" => RedactionPolicy::Strict,
            _ => RedactionPolicy::Redact,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RedactionPolicy::None => "none",
            RedactionPolicy::Redact => "redact",
            RedactionPolicy::Strict => "strict",
        }
    }
}

// Order matters when patterns can overlap. We apply the most specific first
// so that, for example, an Anthropic key (`sk-ant-...`) is tagged as such
// before the broader `sk-...` rule sees it.

static ANTHROPIC_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sk-ant-[A-Za-z0-9_\-]{20,}").unwrap());

static OPENAI_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"sk-[A-Za-z0-9]{20,}").unwrap());

static GITHUB_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"gh[pousr]_[A-Za-z0-9]{36}").unwrap());

static AWS_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").unwrap());

static SLACK_TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"xox[bpoa]-[A-Za-z0-9\-]+").unwrap());

static JWT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_=\-]{10,}\.eyJ[A-Za-z0-9_=\-]{10,}\.[A-Za-z0-9_=\-]+").unwrap()
});

static EMAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}").unwrap());

static SSN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap());

// Credit-card-shaped: 13–19 digits with optional spaces or dashes after each group of four.
// We validate every match with Luhn before redacting so things like phone numbers,
// session IDs, etc. don't get a false positive.
static CC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{4}[ \-]?\d{4}[ \-]?\d{4}[ \-]?\d{4}\b").unwrap());

/// Scrub a single string by replacing each detected secret/PII pattern with
/// a typed `[REDACTED:<type>]` token.
///
/// `RedactionPolicy::None` is a no-op. `Redact` and `Strict` apply the full
/// pattern set; the strict-vs-redact difference is enforced at the event
/// level (see [`redact_event`]) — both call into here.
pub fn redact_string(input: &str, policy: RedactionPolicy) -> String {
    if matches!(policy, RedactionPolicy::None) {
        return input.to_string();
    }
    if input.is_empty() {
        return String::new();
    }

    // Token patterns first (specific → general).
    let mut s = ANTHROPIC_KEY_RE
        .replace_all(input, "[REDACTED:anthropic_key]")
        .into_owned();
    s = OPENAI_KEY_RE
        .replace_all(&s, "[REDACTED:openai_key]")
        .into_owned();
    s = GITHUB_TOKEN_RE
        .replace_all(&s, "[REDACTED:github_token]")
        .into_owned();
    s = AWS_KEY_RE.replace_all(&s, "[REDACTED:aws_key]").into_owned();
    s = SLACK_TOKEN_RE
        .replace_all(&s, "[REDACTED:slack_token]")
        .into_owned();
    s = JWT_RE.replace_all(&s, "[REDACTED:jwt]").into_owned();

    // Credit cards require a Luhn check; can't be expressed purely in regex.
    s = redact_credit_cards(&s);

    // PII patterns.
    s = EMAIL_RE.replace_all(&s, "[REDACTED:email]").into_owned();
    s = SSN_RE.replace_all(&s, "[REDACTED:ssn]").into_owned();

    // Sensitive file paths — reuse the existing classifier so the policy
    // list stays in one place.
    s = redact_sensitive_paths(&s);

    s
}

fn redact_credit_cards(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0;
    for m in CC_RE.find_iter(input) {
        let matched = &input[m.start()..m.end()];
        let digits: String = matched.chars().filter(|c| c.is_ascii_digit()).collect();
        if luhn_check(&digits) {
            result.push_str(&input[last_end..m.start()]);
            result.push_str("[REDACTED:credit_card]");
            last_end = m.end();
        }
    }
    result.push_str(&input[last_end..]);
    result
}

/// Standard Luhn (mod-10) checksum. Accepts 13–19 digit numbers (the range
/// covered by every major card issuer).
pub fn luhn_check(digits: &str) -> bool {
    let len = digits.len();
    if !(13..=19).contains(&len) {
        return false;
    }
    let mut sum: u32 = 0;
    let mut alt = false;
    for c in digits.chars().rev() {
        let mut d = match c.to_digit(10) {
            Some(d) => d,
            None => return false,
        };
        if alt {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
        alt = !alt;
    }
    sum.is_multiple_of(10)
}

fn redact_sensitive_paths(input: &str) -> String {
    let mut result = input.to_string();
    let paths = extract_file_paths(&result);
    for p in paths {
        if is_sensitive_file(&p).is_some() {
            // `replace` is a substring match — fine here because the extracted
            // path string is what we found in `result` verbatim.
            result = result.replace(&p, "[REDACTED:sensitive_file]");
        }
    }
    result
}

/// Walk a `serde_json::Value` recursively and redact every string in place.
/// Structure (keys, numbers, bools, nulls) is preserved.
pub fn redact_json_value(value: &mut serde_json::Value, policy: RedactionPolicy) {
    if matches!(policy, RedactionPolicy::None) {
        return;
    }
    match value {
        serde_json::Value::String(s) => {
            let redacted = redact_string(s, policy);
            *s = redacted;
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                redact_json_value(v, policy);
            }
        }
        serde_json::Value::Object(map) => {
            for (_k, v) in map.iter_mut() {
                redact_json_value(v, policy);
            }
        }
        _ => {}
    }
}

/// Apply the redaction policy to an event in place.
///
/// `system_user` and `system_uid` are NOT redacted — knowing WHO acted is the
/// entire point of an audit log. Anything else carrying free-form text
/// (`description`, `metadata`) is scrubbed.
pub fn redact_event(event: &mut AuditEvent, policy: RedactionPolicy) {
    match policy {
        RedactionPolicy::None => {}
        RedactionPolicy::Redact => {
            event.description = redact_string(&event.description, policy);
            redact_json_value(&mut event.metadata, policy);
        }
        RedactionPolicy::Strict => {
            event.description = event_type_default_description(&event.event_type).to_string();
            event.metadata = serde_json::Value::Object(serde_json::Map::new());
        }
    }
}

/// Fixed, low-cardinality description string for each event type. Used by
/// `RedactionPolicy::Strict` so that even the description carries no
/// caller-supplied data.
pub fn event_type_default_description(t: &AuditEventType) -> &'static str {
    use AuditEventType::*;
    match t {
        CommandExecuted => "command_executed",
        FileRead => "file_read",
        FileWrite => "file_write",
        FileDelete => "file_delete",
        SecretAccess => "secret_access",
        PrivateKeyAccess => "private_key_access",
        PiiDetected => "pii_detected",
        NetworkAccess => "network_access",
        ProcessSpawn => "process_spawn",
        EnvVarAccess => "env_var_access",
        BrowserNavigation => "browser_navigation",
        BrowserJsExecution => "browser_js_execution",
        RpcCall => "rpc_call",
        SandboxToggle => "sandbox_toggle",
        SandboxViolation => "sandbox_violation",
        SandboxPolicyChange => "sandbox_policy_change",
        AgentInvocation => "agent_invocation",
        UserPrompt => "user_prompt",
        ClaudeAppChat => "claude_app_chat",
        QueueTaskSubmitted => "queue_task_submitted",
        QueueTaskStarted => "queue_task_started",
        QueueTaskCompleted => "queue_task_completed",
        QueueTaskFailed => "queue_task_failed",
        QueueTaskCancelled => "queue_task_cancelled",
        QueueModelSelected => "queue_model_selected",
        Custom(_) => "custom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditLog, AuditSeverity, canonical_json_for_signing, compute_hmac, verify_events};
    use chrono::Utc;
    use uuid::Uuid;

    // ─────────── policy parsing ───────────

    #[test]
    fn policy_from_config_value_known() {
        assert_eq!(RedactionPolicy::from_config_value("none"), RedactionPolicy::None);
        assert_eq!(RedactionPolicy::from_config_value("redact"), RedactionPolicy::Redact);
        assert_eq!(RedactionPolicy::from_config_value("strict"), RedactionPolicy::Strict);
        // case-insensitive
        assert_eq!(RedactionPolicy::from_config_value("STRICT"), RedactionPolicy::Strict);
        assert_eq!(RedactionPolicy::from_config_value("  redact  "), RedactionPolicy::Redact);
    }

    #[test]
    fn policy_from_config_unknown_falls_back_to_redact() {
        // Important: a typo MUST NOT silently disable redaction.
        assert_eq!(RedactionPolicy::from_config_value("verbose"), RedactionPolicy::Redact);
        assert_eq!(RedactionPolicy::from_config_value(""), RedactionPolicy::Redact);
    }

    #[test]
    fn policy_default_is_redact() {
        assert_eq!(RedactionPolicy::default(), RedactionPolicy::Redact);
    }

    // ─────────── pattern matching ───────────

    #[test]
    fn redact_email_in_description() {
        let s = redact_string("contact me at alice@example.com please", RedactionPolicy::Redact);
        assert!(s.contains("[REDACTED:email]"), "got: {s}");
        assert!(!s.contains("alice@example.com"));
    }

    #[test]
    fn redact_ssn() {
        let s = redact_string("SSN is 123-45-6789 here", RedactionPolicy::Redact);
        assert!(s.contains("[REDACTED:ssn]"));
        assert!(!s.contains("123-45-6789"));
    }

    #[test]
    fn redact_anthropic_key_not_mis_matched_as_openai() {
        // Anthropic keys begin with `sk-ant-` which is also a prefix of the
        // generic `sk-` OpenAI pattern; must be tagged as anthropic.
        let s = redact_string(
            "key is sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890",
            RedactionPolicy::Redact,
        );
        assert!(s.contains("[REDACTED:anthropic_key]"), "got: {s}");
        assert!(!s.contains("[REDACTED:openai_key]"));
        assert!(!s.contains("sk-ant-"));
    }

    #[test]
    fn redact_openai_key() {
        let s = redact_string(
            "OpenAI key sk-abcdefghijklmnopqrstuvwxyz1234",
            RedactionPolicy::Redact,
        );
        assert!(s.contains("[REDACTED:openai_key]"));
    }

    #[test]
    fn redact_github_token_all_variants() {
        for prefix in &["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
            let token = format!("{prefix}{}", "a".repeat(36));
            let s = redact_string(&format!("token: {token}"), RedactionPolicy::Redact);
            assert!(
                s.contains("[REDACTED:github_token]"),
                "{prefix} token not redacted: {s}"
            );
        }
    }

    #[test]
    fn redact_aws_access_key() {
        let s = redact_string("aws AKIAIOSFODNN7EXAMPLE creds", RedactionPolicy::Redact);
        assert!(s.contains("[REDACTED:aws_key]"));
    }

    #[test]
    fn redact_slack_bot_token() {
        let s = redact_string(
            "slack xoxb-1234-5678-abcdefghij token",
            RedactionPolicy::Redact,
        );
        assert!(s.contains("[REDACTED:slack_token]"));
    }

    #[test]
    fn redact_jwt() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dQw4w9WgXcQ";
        let s = redact_string(&format!("auth: {jwt}"), RedactionPolicy::Redact);
        assert!(s.contains("[REDACTED:jwt]"), "got: {s}");
    }

    #[test]
    fn redact_sensitive_file_path() {
        let s = redact_string("agent read /home/user/.env for config", RedactionPolicy::Redact);
        assert!(s.contains("[REDACTED:sensitive_file]"), "got: {s}");
        assert!(!s.contains("/home/user/.env"));
    }

    #[test]
    fn redact_ssh_private_key_path() {
        let s = redact_string("loaded ~/.ssh/id_rsa", RedactionPolicy::Redact);
        assert!(s.contains("[REDACTED:sensitive_file]"));
        assert!(!s.contains("id_rsa"));
    }

    // ─────────── credit card / Luhn ───────────

    #[test]
    fn redact_valid_credit_card() {
        // Valid test PAN (Visa test card): 4242 4242 4242 4242
        let s = redact_string("card: 4242 4242 4242 4242 expires", RedactionPolicy::Redact);
        assert!(s.contains("[REDACTED:credit_card]"), "got: {s}");
        assert!(!s.contains("4242 4242"));
    }

    #[test]
    fn credit_card_luhn_check_rejects_invalid() {
        // Same shape, but the last digit is wrong → Luhn fails → must NOT be redacted.
        let s = redact_string("not a card: 4242 4242 4242 4241", RedactionPolicy::Redact);
        assert!(
            !s.contains("[REDACTED:credit_card]"),
            "Luhn-invalid sequence was wrongly redacted: {s}"
        );
        assert!(s.contains("4242 4242 4242 4241"));
    }

    #[test]
    fn luhn_basic_cases() {
        assert!(luhn_check("4242424242424242")); // valid Visa test
        assert!(luhn_check("5555555555554444")); // valid Mastercard test
        assert!(luhn_check("378282246310005")); // 15-digit Amex test
        assert!(!luhn_check("4242424242424241"));
        assert!(!luhn_check("1234567890123456"));
        assert!(!luhn_check("0000")); // too short
    }

    // ─────────── policy levels ───────────

    #[test]
    fn none_policy_preserves_verbatim() {
        let original = "email alice@example.com SSN 123-45-6789 key sk-ant-abcdef0123456789xyzAB";
        let s = redact_string(original, RedactionPolicy::None);
        assert_eq!(s, original);
    }

    #[test]
    fn strict_policy_strips_description_and_metadata() {
        let mut event = make_test_event(
            AuditEventType::CommandExecuted,
            "raw command: cat /home/user/.env",
            serde_json::json!({"cmd": "cat /home/user/.env", "out": "user@example.com"}),
        );

        redact_event(&mut event, RedactionPolicy::Strict);

        assert_eq!(event.description, "command_executed");
        // metadata should be an empty object — no caller content survives.
        let obj = event.metadata.as_object().expect("metadata should be object");
        assert!(obj.is_empty(), "strict policy must clear metadata: {:?}", obj);
    }

    #[test]
    fn redact_policy_preserves_metadata_structure() {
        let mut event = make_test_event(
            AuditEventType::FileRead,
            "read /home/user/.env",
            serde_json::json!({
                "path": "/home/user/.env",
                "size": 1024,
                "tags": ["secret", "user@example.com"],
            }),
        );

        redact_event(&mut event, RedactionPolicy::Redact);

        assert!(event.description.contains("[REDACTED:sensitive_file]"));
        let obj = event.metadata.as_object().expect("metadata should be object");
        // structure preserved: keys and types intact
        assert!(obj.contains_key("path"));
        assert!(obj.contains_key("size"));
        assert!(obj.contains_key("tags"));
        assert_eq!(obj.get("size").and_then(|v| v.as_i64()), Some(1024));
        // string values inside scrubbed
        assert!(obj.get("path").and_then(|v| v.as_str()).unwrap().contains("[REDACTED:sensitive_file]"));
        let tags = obj.get("tags").and_then(|v| v.as_array()).unwrap();
        assert!(tags.iter().any(|v| v.as_str().unwrap().contains("[REDACTED:email]")));
    }

    #[test]
    fn redact_metadata_nested_strings() {
        let mut v = serde_json::json!({
            "outer": {
                "inner": "ping alice@example.com",
                "items": ["sk-ant-abcdefghijklmnopqrstuvwxyz0123", 42, true],
            }
        });
        redact_json_value(&mut v, RedactionPolicy::Redact);

        let s = v.to_string();
        assert!(s.contains("[REDACTED:email]"), "{s}");
        assert!(s.contains("[REDACTED:anthropic_key]"), "{s}");
        assert!(s.contains("42"), "non-string scalars must be preserved: {s}");
        assert!(s.contains("true"), "booleans must be preserved: {s}");
    }

    #[test]
    fn redact_event_does_not_touch_system_user() {
        let mut event = make_test_event(
            AuditEventType::CommandExecuted,
            "ran cmd as alice@example.com",
            serde_json::json!({}),
        );
        event.system_user = Some("alice".to_string());
        event.system_uid = Some(501);

        redact_event(&mut event, RedactionPolicy::Strict);

        // even strict policy keeps system identity — we need to know WHO acted
        assert_eq!(event.system_user.as_deref(), Some("alice"));
        assert_eq!(event.system_uid, Some(501));
    }

    // ─────────── chain + HMAC over redacted form ───────────

    #[test]
    fn hash_chain_works_over_redacted_events() {
        let mut log = AuditLog::new(100).with_redaction_policy(RedactionPolicy::Redact);
        let ws = Uuid::new_v4();

        log.log(ws, None, AuditEventType::FileRead, AuditSeverity::Info,
            "read /home/user/.env at user@example.com", serde_json::json!({"path": "/home/user/.env"}));
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "cmd: cat ~/.ssh/id_rsa", serde_json::json!({"cmd": "cat ~/.ssh/id_rsa"}));
        log.log(ws, None, AuditEventType::FileRead, AuditSeverity::Info,
            "ok", serde_json::json!({}));

        let events = log.all();
        assert!(events[0].prev_hash.is_empty(), "first event has empty prev_hash");
        // None of the descriptions should still contain the cleartext.
        for e in events {
            assert!(!e.description.contains("user@example.com"), "leak in: {}", e.description);
            assert!(!e.description.contains("/home/user/.env"));
            assert!(!e.description.contains("id_rsa"));
        }

        // Chain must verify (unsigned path: hash of canonical_json of previous).
        let r = log.verify_integrity();
        assert!(r.verified, "redacted chain must verify, got {r:?}");
    }

    #[test]
    fn hmac_works_over_redacted_events() {
        let key = [11u8; 32];
        let mut log = AuditLog::new(100)
            .with_signing_key(key)
            .with_redaction_policy(RedactionPolicy::Redact);

        let ws = Uuid::new_v4();
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "user@example.com ran cat /home/user/.env",
            serde_json::json!({"key": "sk-ant-abcdefghijklmnopqrstuvwxyz0123"}));
        log.log(ws, None, AuditEventType::FileRead, AuditSeverity::Info,
            "second event", serde_json::json!({}));

        // HMAC was computed over the redacted form, so verification must pass
        // and the on-disk form must have no cleartext.
        let r = log.verify_integrity();
        assert!(r.verified, "{r:?}");

        let first = &log.all()[0];
        assert!(!first.description.contains("user@example.com"));
        assert!(!first.description.contains("/home/user/.env"));
        assert!(!first.metadata.to_string().contains("sk-ant-"));

        // Sanity: independent recompute of the HMAC matches what we stored.
        let recomputed = compute_hmac(first, &key);
        assert_eq!(first.hmac.as_deref(), Some(recomputed.as_str()));

        // Tampering with the stored description AFTER redaction still trips the HMAC.
        let mut tampered = log.all().to_vec();
        tampered[0].description = "attacker rewrote".to_string();
        let r = verify_events(&tampered, Some(&key));
        assert!(!r.verified);
    }

    #[test]
    fn strict_policy_clears_metadata_but_chain_holds() {
        let mut log = AuditLog::new(100).with_redaction_policy(RedactionPolicy::Strict);
        let ws = Uuid::new_v4();

        log.log(ws, None, AuditEventType::SecretAccess, AuditSeverity::Alert,
            "alice opened /home/user/.env (super secret!)",
            serde_json::json!({"path": "/home/user/.env", "size": 4096}));

        let first = &log.all()[0];
        assert_eq!(first.description, "secret_access");
        assert!(first.metadata.as_object().unwrap().is_empty());

        let r = log.verify_integrity();
        assert!(r.verified, "{r:?}");
    }

    #[test]
    fn none_policy_through_audit_log_is_verbatim() {
        let mut log = AuditLog::new(100).with_redaction_policy(RedactionPolicy::None);
        let ws = Uuid::new_v4();
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "raw: alice@example.com", serde_json::json!({"path": "/home/user/.env"}));
        let e = &log.all()[0];
        assert!(e.description.contains("alice@example.com"));
        assert!(e.metadata.to_string().contains("/home/user/.env"));
    }

    #[test]
    fn explicit_hmac_event_is_not_redacted() {
        // A signed event forwarded from a remote (already has an hmac) must be
        // recorded as-is, because redacting it would invalidate the existing
        // signature. The local policy applies only to events we sign ourselves.
        let key = [3u8; 32];
        let mut log = AuditLog::new(10)
            .with_signing_key(key)
            .with_redaction_policy(RedactionPolicy::Strict);

        let mut forwarded = make_test_event(
            AuditEventType::CommandExecuted,
            "forwarded raw description with alice@example.com",
            serde_json::json!({"raw": "secret"}),
        );
        // Pretend a peer signed it under some other policy.
        forwarded.hmac = Some("forwarded-signature".to_string());

        log.record(forwarded);

        let stored = &log.all()[0];
        assert_eq!(stored.hmac.as_deref(), Some("forwarded-signature"));
        // Description preserved (not strict-stripped) because we didn't sign it.
        assert!(stored.description.contains("alice@example.com"),
            "forwarded event description was overwritten: {}", stored.description);
    }

    // ─────────── end-to-end on-disk negative test ───────────

    #[test]
    fn end_to_end_no_cleartext_on_disk_under_redact() {
        // Drive several events with every supported pattern, serialize the
        // way the on-disk JSONL store does, and assert that NONE of the
        // cleartext fragments appear in the bytes.
        let mut log = AuditLog::new(100)
            .with_signing_key([1u8; 32])
            .with_redaction_policy(RedactionPolicy::Redact);
        let ws = Uuid::new_v4();

        let cleartext_samples = [
            "alice@example.com",
            "123-45-6789",
            "sk-ant-abcdefghijklmnopqrstuvwxyz0123",
            "sk-abcdefghijklmnopqrstuvwxyz0123",
            "ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "AKIAIOSFODNN7EXAMPLE",
            "xoxb-1234-5678-abcdefghij",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dQw4w9WgXcQ",
            "4242 4242 4242 4242",
            "/home/user/.env",
            "~/.ssh/id_rsa",
        ];

        for (i, sample) in cleartext_samples.iter().enumerate() {
            log.log(
                ws,
                None,
                AuditEventType::CommandExecuted,
                AuditSeverity::Info,
                format!("event {i}: contains {sample}"),
                serde_json::json!({"raw": sample, "nested": {"v": sample}}),
            );
        }

        // Serialize as the audit store would (JSON per event).
        let mut all_bytes = String::new();
        for e in log.all() {
            all_bytes.push_str(&serde_json::to_string(e).unwrap());
            all_bytes.push('\n');
        }

        for sample in &cleartext_samples {
            assert!(
                !all_bytes.contains(sample),
                "cleartext sample {sample:?} leaked into on-disk form:\n{all_bytes}"
            );
        }
        // Chain still verifies.
        assert!(log.verify_integrity().verified);
    }

    #[test]
    fn end_to_end_no_cleartext_on_disk_under_strict() {
        let mut log = AuditLog::new(100).with_redaction_policy(RedactionPolicy::Strict);
        let ws = Uuid::new_v4();
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "alice@example.com 4242 4242 4242 4242",
            serde_json::json!({"raw": "sk-ant-abcdefghijklmnopqrstuvwxyz0123"}));

        let mut bytes = String::new();
        for e in log.all() {
            bytes.push_str(&serde_json::to_string(e).unwrap());
            bytes.push('\n');
        }
        for fragment in &["alice@example.com", "4242", "sk-ant-"] {
            assert!(!bytes.contains(fragment), "leak: {fragment} in {bytes}");
        }
    }

    // ─────────── helpers ───────────

    fn make_test_event(
        event_type: AuditEventType,
        description: &str,
        metadata: serde_json::Value,
    ) -> AuditEvent {
        AuditEvent {
            id: Uuid::nil(),
            timestamp: Utc::now(),
            workspace_id: Uuid::nil(),
            panel_id: None,
            event_type,
            severity: AuditSeverity::Info,
            description: description.to_string(),
            metadata,
            agent_name: None,
            system_user: None,
            system_uid: None,
            prev_hash: String::new(),
            hmac: None,
        }
    }

    #[test]
    fn redact_string_empty_is_noop() {
        assert_eq!(redact_string("", RedactionPolicy::Redact), "");
        assert_eq!(redact_string("", RedactionPolicy::Strict), "");
    }

    #[test]
    fn event_type_default_description_covers_known_variants() {
        // Spot-check a few — every variant must yield some non-empty string.
        for t in [
            AuditEventType::CommandExecuted,
            AuditEventType::PiiDetected,
            AuditEventType::SandboxViolation,
            AuditEventType::Custom("anything".to_string()),
        ] {
            let d = event_type_default_description(&t);
            assert!(!d.is_empty());
            assert!(!d.contains(' '), "must be a single token: {d}");
        }
    }

    #[test]
    fn canonical_json_for_redacted_event_is_deterministic() {
        // Sanity: redacting an event and serializing twice yields identical bytes,
        // which is what makes the HMAC reproducible.
        let mut e1 = make_test_event(
            AuditEventType::FileRead,
            "user@example.com read /home/user/.env",
            serde_json::json!({"path": "/home/user/.env"}),
        );
        let mut e2 = e1.clone();
        redact_event(&mut e1, RedactionPolicy::Redact);
        redact_event(&mut e2, RedactionPolicy::Redact);
        assert_eq!(
            canonical_json_for_signing(&e1).unwrap(),
            canonical_json_for_signing(&e2).unwrap()
        );
    }
}
