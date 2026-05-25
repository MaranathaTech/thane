//! `thane enterprise` — manage enterprise audit policy enrollment.
//!
//! Self-service enrollment flow for small teams that don't have full MDM
//! (Jamf/Intune/Munki) deployed but still want their thane installs to ship
//! audit events to a central Loki + Grafana.
//!
//! Subcommands:
//! - `enroll <url> [--token TOKEN]` — fetch policy + Loki bearer token from
//!   an enrollment server, write the policy to the root-owned policy file,
//!   and stash the token in the platform secret store.
//! - `status` — read-only view of the currently active policy.
//! - `leave` — delete the policy file + secret. Requires sudo; warn that MDM
//!   may re-push it on the next sync.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::json;

use thane_core::audit::{AuditEvent, AuditEventType, AuditLog, AuditSeverity};
use thane_core::policy::{self, EnterprisePolicy};
use thane_persist::audit_store::AuditStore;
use thane_platform::default_secret_store;

#[derive(Subcommand)]
pub enum EnterpriseCommand {
    /// Enroll this device against an enterprise audit-policy server.
    ///
    /// POSTs `{token, hostname, system_user}` to `<enrollment-url>` and expects
    /// a JSON response with a `policy` (JSON `EnterprisePolicy`), a
    /// `bearer_token` (stashed in the platform secret store), and a `tenant`
    /// id (echoed in the enrollment audit event).
    Enroll {
        /// The enterprise's enrollment endpoint URL (HTTPS).
        url: String,
        /// One-time enrollment token issued by the admin.
        #[arg(short, long)]
        token: Option<String>,
        /// Skip the confirmation prompt before writing the policy.
        #[arg(long)]
        yes: bool,
    },
    /// Show the currently active enterprise policy (if any).
    Status,
    /// Remove the locally-installed enterprise policy + secret.
    ///
    /// Requires sudo (the policy file is root-owned). MDM systems may re-push
    /// the policy on their next sync — leaving is therefore best-effort and
    /// not a guarantee of removal.
    Leave {
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

/// Server response shape. Liberal field set — only `policy` is required.
#[derive(Debug, Deserialize)]
struct EnrollResponse {
    policy: EnterprisePolicy,
    #[serde(default)]
    bearer_token: Option<String>,
    #[serde(default)]
    tenant: Option<String>,
}

/// Request body posted to the enrollment endpoint.
#[derive(Debug, Serialize)]
struct EnrollRequest<'a> {
    token: Option<&'a str>,
    hostname: String,
    system_user: String,
}

impl EnterpriseCommand {
    pub async fn execute(self, _socket_path: &str) -> Result<()> {
        // These commands intentionally do NOT talk to the running daemon
        // socket — they modify root-owned filesystem state and need to work
        // even before/after the daemon is up.
        match self {
            EnterpriseCommand::Enroll { url, token, yes } => enroll(&url, token.as_deref(), yes),
            EnterpriseCommand::Status => status(),
            EnterpriseCommand::Leave { yes } => leave(yes),
        }
    }
}

fn policy_file_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        thane_platform::MacosDirs.policy_file_path()
    }
    #[cfg(target_os = "linux")]
    {
        thane_platform::LinuxDirs.policy_file_path()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        PathBuf::from("/tmp/thane-policy.json")
    }
}

fn enroll(url: &str, token: Option<&str>, yes: bool) -> Result<()> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        bail!("enrollment URL must start with https:// (or http:// for local testing)");
    }
    if url.starts_with("http://") && !url.contains("localhost") && !url.contains("127.0.0.1") {
        eprintln!("warning: non-localhost http:// enrollment URL — credentials will be sent in cleartext");
    }

    let body = EnrollRequest {
        token,
        hostname: whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string()),
        system_user: whoami::username(),
    };
    let body_json = serde_json::to_string(&body)?;

    eprintln!("Posting enrollment request to {url} …");
    let resp = ureq::post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .send(body_json.as_bytes())
        .map_err(|e| anyhow::anyhow!("enrollment request failed: {e}"))?;

    let status_code = resp.status().as_u16();
    let mut resp = resp;
    let resp_body = resp
        .body_mut()
        .read_to_string()
        .context("read enrollment response body")?;
    if !(200..300).contains(&status_code) {
        bail!(
            "enrollment server returned HTTP {status_code}: {snippet}",
            snippet = resp_body.chars().take(500).collect::<String>(),
        );
    }
    let parsed: EnrollResponse = serde_json::from_str(&resp_body)
        .with_context(|| format!("parse enrollment response: {resp_body}"))?;

    // Show the user what they're about to commit to.
    let policy = parsed.policy;
    println!();
    println!("Enrollment response received:");
    println!("  issued_by   : {}", policy.issued_by);
    println!("  issued_at   : {}", policy.issued_at);
    println!("  policy_ver  : {}", policy.policy_version);
    println!("  locked keys : {}", policy.locked_keys.len());
    for (k, v) in &policy.locked_keys {
        println!("    - {k} = {v}");
    }
    if let Some(banner) = &policy.ui_banner {
        println!("  ui banner   : {banner}");
    }
    if let Some(tenant) = parsed.tenant.as_deref() {
        println!("  tenant      : {tenant}");
    }
    println!();

    if !yes && !confirm("Apply this enterprise policy to THIS device?")? {
        eprintln!("aborted; no changes made.");
        return Ok(());
    }

    // Stash the bearer token BEFORE the policy lands — that way the sink can
    // come up cleanly on the next daemon restart without a dangling
    // "secret not found" log.
    if let Some(token) = parsed.bearer_token.as_deref() {
        let store = default_secret_store();
        let secret_id = policy
            .locked_keys
            .get("audit-sink-loki-auth-secret-id")
            .cloned()
            .unwrap_or_else(|| "thane-loki-token".to_string());
        store
            .set(&secret_id, token.as_bytes())
            .map_err(|e| anyhow::anyhow!("store loki bearer token: {e}"))?;
        eprintln!("Stored Loki bearer token under secret id '{secret_id}'.");
    }

    // Write the policy file. Wrapped so we can fall back to user config on
    // permission denied.
    let path = policy_file_path();
    let policy_bytes = serde_json::to_vec_pretty(&policy)?;
    match write_policy_file(&path, &policy_bytes) {
        Ok(()) => {
            eprintln!("Wrote enterprise policy to {}.", path.display());
        }
        Err(WriteErr::PermissionDenied) => {
            eprintln!(
                "warning: cannot write {} (permission denied). \
                 Re-run via sudo to enforce the policy at the system level, \
                 or accept the user-level fallback below (which the user can disable).",
                path.display()
            );
            if !yes && !confirm("Write the policy keys to the USER config instead?")? {
                bail!("policy file write declined; no changes persisted to disk");
            }
            apply_to_user_config(&policy)?;
            eprintln!(
                "Applied policy keys to user config. The user can disable any of these \
                 settings; only the root-owned policy file at {} enforces enterprise lock.",
                path.display()
            );
        }
        Err(WriteErr::Io(e)) => return Err(e.into()),
    }

    // Emit an audit event so the SOC sees the enrollment land.
    let tenant_for_event = parsed
        .tenant
        .as_deref()
        .or_else(|| policy.locked_keys.get("audit-sink-loki-tenant").map(|s| s.as_str()))
        .unwrap_or("unknown");
    emit_audit_event(
        "enterprise_enrolled",
        AuditSeverity::Alert,
        format!(
            "Enterprise policy enrolled (issued_by={}, tenant={tenant_for_event})",
            policy.issued_by
        ),
        json!({
            "tenant": tenant_for_event,
            "issued_by": policy.issued_by,
            "issued_at": policy.issued_at,
            "policy_version": policy.policy_version,
            "locked_keys": policy.locked_keys.keys().cloned().collect::<Vec<_>>(),
        }),
    )?;

    eprintln!("Enrollment complete. Restart thane to apply the new policy.");
    Ok(())
}

fn status() -> Result<()> {
    match policy::load_for_platform()? {
        None => {
            println!("No enterprise policy is active on this device.");
            Ok(())
        }
        Some(p) => {
            println!("Enterprise policy active:");
            println!("  policy_version : {}", p.policy_version);
            println!("  issued_by      : {}", p.issued_by);
            println!("  issued_at      : {}", p.issued_at);
            if let Some(b) = p.ui_banner.as_deref() {
                println!("  ui_banner      : {b}");
            }
            println!("  locked_keys    :");
            let mut keys: Vec<_> = p.locked_keys.iter().collect();
            keys.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in keys {
                println!("    {k} = {v}");
            }
            Ok(())
        }
    }
}

fn leave(yes: bool) -> Result<()> {
    let path = policy_file_path();
    let Some(policy) = policy::load_for_platform()? else {
        println!("No active enterprise policy. Nothing to remove.");
        return Ok(());
    };

    println!(
        "About to remove the enterprise policy issued by {:?}.",
        policy.issued_by
    );
    println!(
        "If your organization manages this device via MDM, the policy may be \
         re-pushed on the next sync. This action is best-effort."
    );
    if !yes && !confirm("Proceed with removing the policy?")? {
        eprintln!("aborted; policy left in place.");
        return Ok(());
    }

    match std::fs::remove_file(&path) {
        Ok(()) => eprintln!("Removed {}.", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => bail!(
            "permission denied removing {}; re-run via sudo",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Could be a plist-only deployment; ignore.
            eprintln!(
                "{} not present; if your org uses Managed Preferences, contact IT to lift the policy.",
                path.display()
            );
        }
        Err(e) => return Err(e.into()),
    }

    // Best-effort secret removal — the secret-store entry may have been set
    // under a custom secret id; we honor the now-removed policy's value if
    // present, else fall back to the default.
    let secret_id = policy
        .locked_keys
        .get("audit-sink-loki-auth-secret-id")
        .cloned()
        .unwrap_or_else(|| "thane-loki-token".to_string());
    let store = default_secret_store();
    if let Err(e) = store.delete(&secret_id) {
        eprintln!(
            "warning: could not delete loki bearer secret '{secret_id}': {e} \
             (the policy file is gone; this is cosmetic)"
        );
    }

    emit_audit_event(
        "enterprise_unenrolled",
        AuditSeverity::Alert,
        format!(
            "Enterprise policy removed (issued_by={})",
            policy.issued_by
        ),
        json!({
            "issued_by": policy.issued_by,
            "policy_version": policy.policy_version,
            "removed_keys": policy.locked_keys.keys().cloned().collect::<Vec<_>>(),
        }),
    )?;

    eprintln!("Unenrollment complete. Restart thane to drop the locked-key overrides.");
    Ok(())
}

#[derive(Debug)]
enum WriteErr {
    PermissionDenied,
    Io(std::io::Error),
}

impl From<std::io::Error> for WriteErr {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            WriteErr::PermissionDenied
        } else {
            WriteErr::Io(e)
        }
    }
}

fn write_policy_file(path: &Path, bytes: &[u8]) -> Result<(), WriteErr> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    // Mode 0644: root-owned but world-readable so the daemon (running as the
    // logged-in user) can read it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&tmp) {
            let mut perms = meta.permissions();
            perms.set_mode(0o644);
            let _ = std::fs::set_permissions(&tmp, perms);
        }
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// User-config fallback when we can't write the root-owned policy file.
/// The keys ARE applied, but the user can edit them out — call this only
/// when the operator has explicitly accepted that downside.
fn apply_to_user_config(policy: &EnterprisePolicy) -> Result<()> {
    let mut cfg = thane_core::config::Config::load_default();
    for (k, v) in &policy.locked_keys {
        cfg.set(k, v);
    }
    cfg.save()
        .map_err(|e| anyhow::anyhow!("save user config: {e}"))?;
    Ok(())
}

/// Emit a Custom audit event to the on-disk audit log. The daemon, if
/// running, picks it up on next read; otherwise it's durably persisted for
/// forensic review.
fn emit_audit_event(
    kind: &str,
    severity: AuditSeverity,
    description: String,
    metadata: serde_json::Value,
) -> Result<()> {
    let dirs = platform_dirs_sessions_dir();
    let config = thane_core::config::Config::load_default();
    let mut log = AuditLog::new(8).with_redaction_policy(config.audit_redaction_policy());
    if config.audit_signing_enabled() {
        let store = default_secret_store();
        if let Ok(key) = thane_core::audit_keys::try_audit_hmac_key(store.as_ref()) {
            log.set_signing_key(key);
        } else {
            tracing::warn!(
                "audit-signing-enabled but no key available; '{kind}' event will be unsigned"
            );
        }
    }
    log.log(
        uuid::Uuid::nil(),
        None,
        AuditEventType::Custom(kind.to_string()),
        severity,
        description,
        metadata,
    );

    let store = AuditStore::new(dirs).with_retention_days(config.audit_retention_days());
    // We only just-appended one event; flush writes all in-memory events.
    if let Some(ev) = log.all().last() {
        store
            .append(ev)
            .map_err(|e| anyhow::anyhow!("write audit event '{kind}': {e}"))?;
    }
    Ok(())
}

fn platform_dirs_sessions_dir() -> PathBuf {
    use thane_platform::traits::PlatformDirs;
    #[cfg(target_os = "macos")]
    {
        thane_platform::MacosDirs.sessions_dir()
    }
    #[cfg(target_os = "linux")]
    {
        thane_platform::LinuxDirs.sessions_dir()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        std::env::temp_dir().join("thane")
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    use std::io::{BufRead, Write as _};
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush().ok();
    let stdin = std::io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    let trimmed = line.trim().to_ascii_lowercase();
    Ok(matches!(trimmed.as_str(), "y" | "yes"))
}

// `Write` import retained to keep the explicit io::Write trait scope inside
// helpers — see `confirm`/`write_policy_file`.
#[allow(dead_code)]
fn _silence_write_warning(mut w: impl Write) -> std::io::Result<()> {
    write!(w, "")
}
// Imports asserted for downstream usage; silence rustc unused-import.
#[allow(dead_code)]
fn _silence_audit_event_unused(_e: &AuditEvent) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn dummy_policy() -> EnterprisePolicy {
        EnterprisePolicy {
            policy_version: 1,
            issued_by: "Acme".into(),
            issued_at: "2026-05-25T00:00:00Z".into(),
            locked_keys: HashMap::from([
                ("audit-sink-loki-enabled".to_string(), "true".to_string()),
                ("audit-sink-loki-tenant".to_string(), "acme-inc".to_string()),
            ]),
            ui_banner: Some("Acme policy active".into()),
        }
    }

    #[test]
    fn write_policy_file_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested/policy.json");
        let bytes = serde_json::to_vec_pretty(&dummy_policy()).unwrap();
        write_policy_file(&path, &bytes).expect("write");
        // Round-trip parse.
        let parsed = policy::load_json_from(&path).expect("load").expect("present");
        assert_eq!(parsed.locked_keys.len(), 2);
        assert_eq!(parsed.issued_by, "Acme");
    }

    #[test]
    fn enroll_response_deserializes() {
        let raw = r#"{
            "policy": {
                "policy_version": 1,
                "issued_by": "X",
                "issued_at": "now",
                "locked_keys": {"a": "1"},
                "ui_banner": null
            },
            "bearer_token": "tok-abc",
            "tenant": "x-corp"
        }"#;
        let parsed: EnrollResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.policy.locked_keys.get("a").map(String::as_str), Some("1"));
        assert_eq!(parsed.bearer_token.as_deref(), Some("tok-abc"));
        assert_eq!(parsed.tenant.as_deref(), Some("x-corp"));
    }

    #[test]
    fn enroll_response_tolerates_missing_optional_fields() {
        let raw = r#"{
            "policy": {
                "policy_version": 1,
                "issued_by": "X",
                "issued_at": "",
                "locked_keys": {}
            }
        }"#;
        let parsed: EnrollResponse = serde_json::from_str(raw).unwrap();
        assert!(parsed.bearer_token.is_none());
        assert!(parsed.tenant.is_none());
    }
}
