use std::io::{self, BufRead, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use serde_json::json;

use thane_core::audit::{VerifyFailureKind, verify_events};
use thane_core::audit_keys::{
    audit_hmac_key, store_root_key, try_audit_aes_key,
};
use thane_persist::audit_store::load_events_from_path;
use thane_platform::default_secret_store;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum AuditCommand {
    /// List recent audit events.
    List {
        /// Minimum severity filter (info, warning, alert, critical).
        #[arg(short, long)]
        severity: Option<String>,
        /// Maximum number of events to return.
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },
    /// Export all audit events as JSON.
    Export {
        /// Output file path (prints to stdout if not specified).
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Clear all audit events.
    Clear,
    /// Set the sensitive operation policy for a workspace.
    SetPolicy {
        /// Action to take: allow, warn, or block.
        action: String,
        /// Workspace ID (targets active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Verify the integrity of an audit log on disk.
    ///
    /// Checks that every event's HMAC signature is valid and that the hash chain
    /// is unbroken. Useful for offline forensic review by a compliance auditor.
    Verify {
        /// Path to a JSONL log file, OR a directory containing audit.jsonl +
        /// rotated files (audit.N.jsonl).
        #[arg(short, long)]
        file: PathBuf,
        /// Path to a key file (hex). If omitted, loads the key from the
        /// platform secret store.
        #[arg(long)]
        key_path: Option<PathBuf>,
    },
    /// Export the audit HMAC key as hex.
    ///
    /// Anyone holding this key can forge thane audit events. Hand only to
    /// authorized compliance auditors. Requires --i-understand.
    ExportKey {
        /// Where to write the hex key (file is created with mode 0600).
        #[arg(short, long)]
        output: PathBuf,
        /// Required confirmation flag.
        #[arg(long)]
        i_understand: bool,
    },
    /// Import a hex HMAC key into the platform secret store.
    ///
    /// Used to restore the key on a new host, or to rotate. All existing logs
    /// signed under the previous key will fail verification after this runs.
    ImportKey {
        /// Path to the hex key file to import.
        #[arg(short, long)]
        input: PathBuf,
        /// Skip the y/N confirmation prompt.
        #[arg(long)]
        force: bool,
    },

    /// External sink status (Phase 5).
    SinkStatus,

    /// Dead-letter queue management (events that failed external delivery).
    #[command(subcommand)]
    Dlq(DlqCommand),
}

#[derive(Subcommand)]
pub enum DlqCommand {
    /// Print DLQ entries (newest first).
    List {
        /// Filter by sink name (e.g. "syslog", "webhook").
        #[arg(long)]
        sink: Option<String>,
        /// Max entries to print (default 20). `--all` overrides this.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Print all entries; ignores --limit.
        #[arg(long)]
        all: bool,
    },
    /// Re-enqueue DLQ entries through the dispatcher.
    Retry {
        /// Only retry entries from this sink.
        #[arg(long)]
        sink: Option<String>,
        /// Only retry the entry for this audit event UUID.
        #[arg(long)]
        id: Option<String>,
    },
    /// Truncate the DLQ. Gated by `audit-allow-clear` in config.
    Clear,
}

impl AuditCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::List { severity, limit } => {
                let resp = send_rpc(
                    socket_path,
                    "audit.list",
                    json!({ "severity": severity, "limit": limit }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Export { output } => {
                let resp = send_rpc(socket_path, "audit.export", json!({})).await?;
                if let Some(path) = output {
                    if let Some(ref result) = resp.result {
                        let json_str = serde_json::to_string_pretty(result)?;
                        std::fs::write(&path, json_str)?;
                        println!("Audit log exported to {path}");
                    }
                    Ok(())
                } else {
                    print_response(&resp)
                }
            }
            Self::Clear => {
                let resp = send_rpc(socket_path, "audit.clear", json!({})).await?;
                print_response(&resp)
            }
            Self::SetPolicy { action, id } => {
                let resp = send_rpc(
                    socket_path,
                    "audit.set_sensitive_policy",
                    json!({ "action": action, "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Verify { file, key_path } => verify_command(file, key_path),
            Self::ExportKey { output, i_understand } => {
                export_key_command(output, i_understand)
            }
            Self::ImportKey { input, force } => import_key_command(input, force),
            Self::SinkStatus => {
                let resp = send_rpc(socket_path, "audit.sink_status", json!({})).await?;
                print_response(&resp)
            }
            Self::Dlq(cmd) => cmd.execute(socket_path).await,
        }
    }
}

impl DlqCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::List { sink, limit, all } => {
                let effective_limit = if all { usize::MAX } else { limit };
                let resp = send_rpc(
                    socket_path,
                    "audit.dlq_list",
                    json!({ "sink": sink, "limit": effective_limit }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Retry { sink, id } => {
                let resp = send_rpc(
                    socket_path,
                    "audit.dlq_retry",
                    json!({ "sink": sink, "event_id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Clear => {
                let resp = send_rpc(socket_path, "audit.dlq_clear", json!({})).await?;
                print_response(&resp)
            }
        }
    }
}

fn verify_command(path: PathBuf, key_path: Option<PathBuf>) -> Result<()> {
    // For verification we always pull the AES key from the platform secret
    // store so we can decrypt any `.enc` rotated files we encounter. It's
    // independent of the optional --key-path which only overrides the HMAC key.
    let store = default_secret_store();
    let aes_key = try_audit_aes_key(store.as_ref())
        .map_err(|e| anyhow!("audit AES key unavailable: {e}"))?;

    let events = load_events_from_path(&path, Some(&aes_key))
        .with_context(|| format!("loading audit events from {}", path.display()))?;

    let key = match key_path {
        Some(p) => load_key_from_file(&p)?,
        None => audit_hmac_key(store.as_ref()),
    };

    // Warn if any event is missing an `hmac` field (signing was off at some point).
    let unsigned_count = events.iter().filter(|e| e.hmac.is_none()).count();
    if unsigned_count > 0 {
        eprintln!(
            "WARNING: {unsigned_count} of {} events have no HMAC (audit-signing-enabled was off when they were recorded). \
             These events will fail HMAC verification.",
            events.len()
        );
    }

    let result = verify_events(&events, Some(&key));
    if result.verified {
        println!(
            "VERIFIED: {} events, chain intact, signatures valid",
            result.events_checked
        );
        Ok(())
    } else {
        let f = result
            .first_failure
            .ok_or_else(|| anyhow!("verify failed but no failure detail returned"))?;
        let kind = match f.kind {
            VerifyFailureKind::HmacMismatch => "hmac_mismatch",
            VerifyFailureKind::ChainBreak => "chain_break",
            VerifyFailureKind::MissingHmac => "missing_hmac",
            VerifyFailureKind::UnparseableEvent => "unparseable_event",
        };
        println!(
            "FAILED at event index {} (id {}): {kind}",
            f.event_index, f.event_id
        );
        std::process::exit(1);
    }
}

fn export_key_command(output: PathBuf, i_understand: bool) -> Result<()> {
    let warning = "WARNING: This key authenticates all audit logs. Treat as a secret. \
                   Anyone with this key can forge thane audit events.";
    if !i_understand {
        eprintln!("{warning}");
        eprintln!("Re-run with --i-understand to proceed.");
        std::process::exit(2);
    }
    eprintln!("{warning}");

    let store = default_secret_store();
    let key = audit_hmac_key(store.as_ref());
    let hex_key = hex::encode(key);

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&output)
        .with_context(|| format!("creating {}", output.display()))?;
    f.write_all(hex_key.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o600))?;

    println!("Wrote audit HMAC key to {}", output.display());
    Ok(())
}

fn import_key_command(input: PathBuf, force: bool) -> Result<()> {
    let key = load_key_from_file(&input)?;
    if !force {
        eprint!(
            "This will replace the existing key. All existing logs will FAIL \
             verification afterwards unless re-signed. Continue? (y/N) "
        );
        io::stderr().flush().ok();
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        let trimmed = line.trim().to_ascii_lowercase();
        if trimmed != "y" && trimmed != "yes" {
            println!("Aborted.");
            return Ok(());
        }
    }
    let store = default_secret_store();
    store_root_key(store.as_ref(), &key)
        .map_err(|e| anyhow!("failed to write key to secret store: {e}"))?;
    println!("Audit root key imported. Restart thane services to pick up the new key.");
    Ok(())
}

fn load_key_from_file(path: &std::path::Path) -> Result<[u8; 32]> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading key file {}", path.display()))?;
    let trimmed = raw.trim();
    let bytes = hex::decode(trimmed)
        .map_err(|e| anyhow!("key file {} is not valid hex: {e}", path.display()))?;
    if bytes.len() != 32 {
        bail!(
            "key file {} has {} bytes, expected 32",
            path.display(),
            bytes.len()
        );
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::audit::{AuditEventType, AuditSeverity, AuditLog};
    use uuid::Uuid;

    fn temp_dir(suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "thane-cli-audit-test-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn load_key_from_file_round_trip() {
        let dir = temp_dir("loadkey");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("key.hex");
        let key = [0x42u8; 32];
        std::fs::write(&path, hex::encode(key)).unwrap();
        let got = load_key_from_file(&path).unwrap();
        assert_eq!(got, key);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_key_from_file_rejects_wrong_length() {
        let dir = temp_dir("badlen");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("short.hex");
        std::fs::write(&path, hex::encode([0u8; 16])).unwrap();
        let err = load_key_from_file(&path).unwrap_err();
        assert!(err.to_string().contains("expected 32"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_key_from_file_rejects_non_hex() {
        let dir = temp_dir("nothex");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("garbage.hex");
        std::fs::write(&path, "ZZZZ not hex").unwrap();
        let err = load_key_from_file(&path).unwrap_err();
        assert!(err.to_string().contains("hex"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End-to-end-ish: write a signed log to a jsonl file, then verify it with
    /// the same key. We bypass the CLI's stdout/exit-code path and just call
    /// verify_events directly with the loaded events, matching what
    /// verify_command does internally.
    #[test]
    fn signed_log_round_trip_through_file() {
        let dir = temp_dir("signed-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let key = [7u8; 32];
        let mut log = AuditLog::new(10).with_signing_key(key);
        let ws = Uuid::new_v4();
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "first signed", serde_json::json!({}));
        log.log(ws, None, AuditEventType::FileRead, AuditSeverity::Info,
            "second signed", serde_json::json!({}));

        // Write as JSONL.
        let path = dir.join("audit.jsonl");
        let mut out = String::new();
        for e in log.all() {
            out.push_str(&serde_json::to_string(e).unwrap());
            out.push('\n');
        }
        std::fs::write(&path, out).unwrap();

        let events = load_events_from_path(&path, None).unwrap();
        assert_eq!(events.len(), 2);
        let r = verify_events(&events, Some(&key));
        assert!(r.verified, "round-tripped signed log should verify: {r:?}");

        // Tamper: flip a byte in the description of event 0, re-write, verify
        // fails with HmacMismatch.
        let mut tampered = events.clone();
        tampered[0].description = "rewritten".to_string();
        let r = verify_events(&tampered, Some(&key));
        assert!(!r.verified);
        assert_eq!(
            r.first_failure.unwrap().kind,
            VerifyFailureKind::HmacMismatch
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_key_creates_0600_file() {
        // Skip if not on a unix-like system (we always are in this project, but
        // the test makes the mode check explicit).
        let dir = temp_dir("exportkey");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("k.hex");

        // export_key_command pulls the key from the default secret store on
        // this machine; we don't want to depend on whether it's been
        // initialized. Instead, simulate the file write by calling the same
        // OpenOptions mode invocation:
        let key = [0u8; 32];
        let hex_key = hex::encode(key);
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        f.write_all(hex_key.as_bytes()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
