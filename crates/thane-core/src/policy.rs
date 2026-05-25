//! Enterprise policy override layer.
//!
//! User config alone is a single source of truth — fine for individual
//! installs, but in an enterprise an IT admin needs to force specific audit
//! settings (where logs go, what redaction applies, whether the user may
//! disable shipping) and have the user unable to disable them.
//!
//! A policy file deployed via MDM (Jamf, Intune, Munki, Ansible, etc.) lives
//! at a root-owned path. When present, the keys it lists ALWAYS override the
//! user's config and the UI surfaces those keys as locked. Removal requires
//! root, so the user cannot disable it from inside the app.
//!
//! Precedence (low → high):
//!   1. Built-in defaults
//!   2. User config (`~/Library/Application Support/thane/config` on macOS,
//!      `~/.config/thane/config` on Linux)
//!   3. **Enterprise policy** (this module)
//!
//! On macOS we also honor Apple's Managed Preferences mechanism at
//! `/Library/Managed Preferences/com.thane.app.plist`. When both that and the
//! JSON policy file are present, Managed Preferences wins per
//! [`merge_policies`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Wire format for a deployed enterprise policy.
///
/// Same shape for the JSON file and (with a one-to-one mapping) the macOS
/// Managed Preferences plist. The `policy_version` field is reserved for
/// future schema migrations and is currently always `1`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnterprisePolicy {
    /// Schema version. Currently always `1`. Reserved for future migrations.
    #[serde(default = "default_policy_version")]
    pub policy_version: u32,
    /// Human-readable name of the issuing organization, e.g.
    /// `"Acme Corp IT Security"`. Surfaced in the UI banner so the operator
    /// knows who set this policy.
    #[serde(default)]
    pub issued_by: String,
    /// ISO-8601 timestamp the policy was issued. Free-form string (we don't
    /// parse it — only display it).
    #[serde(default)]
    pub issued_at: String,
    /// The actual override map: config-key → value. Anything in here is
    /// considered locked: `Config::set` will refuse to mutate it and
    /// `Config::is_locked` returns `true` for the key.
    #[serde(default)]
    pub locked_keys: HashMap<String, String>,
    /// Optional banner to render above settings + audit panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_banner: Option<String>,
}

fn default_policy_version() -> u32 {
    1
}

impl EnterprisePolicy {
    /// Whether this policy was sourced from a file (the caller decides; this
    /// is mostly a convenience for tests/builders).
    pub fn has_lockable_keys(&self) -> bool {
        !self.locked_keys.is_empty()
    }

    /// Look up the locked value for a key, if any.
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.locked_keys.get(key).map(|s| s.as_str())
    }

    /// Whether `key` is locked by this policy.
    pub fn is_locked(&self, key: &str) -> bool {
        self.locked_keys.contains_key(key)
    }
}

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("policy file io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("policy file json parse: {0}")]
    Json(#[from] serde_json::Error),
    #[error("policy file plist parse: {0}")]
    Plist(String),
}

/// Result of merging the JSON policy file with the macOS Managed Preferences
/// plist. The plist's `locked_keys` win on conflict; `ui_banner` falls back
/// to whichever source set it; `issued_by` / `issued_at` prefer the plist
/// when both are non-empty.
pub fn merge_policies(
    json: Option<EnterprisePolicy>,
    plist: Option<EnterprisePolicy>,
) -> Option<EnterprisePolicy> {
    match (json, plist) {
        (None, None) => None,
        (Some(p), None) | (None, Some(p)) => Some(p),
        (Some(mut j), Some(p)) => {
            // Managed Preferences override JSON for every locked key.
            j.locked_keys.extend(p.locked_keys);
            if !p.issued_by.is_empty() {
                j.issued_by = p.issued_by;
            }
            if !p.issued_at.is_empty() {
                j.issued_at = p.issued_at;
            }
            if p.ui_banner.is_some() {
                j.ui_banner = p.ui_banner;
            }
            // Take the higher policy_version so a forward-rev MDM can mark
            // its policy newer without rewriting the JSON file.
            if p.policy_version > j.policy_version {
                j.policy_version = p.policy_version;
            }
            Some(j)
        }
    }
}

/// Parse a JSON policy file.
pub fn parse_json(bytes: &[u8]) -> Result<EnterprisePolicy, PolicyError> {
    let p: EnterprisePolicy = serde_json::from_slice(bytes)?;
    Ok(p)
}

/// Parse a macOS Managed Preferences plist file. On non-macOS this still
/// works because the `plist` crate is cross-platform; we just rarely use it
/// off macOS in production.
#[cfg(target_os = "macos")]
pub fn parse_plist(bytes: &[u8]) -> Result<EnterprisePolicy, PolicyError> {
    let value: plist::Value =
        plist::from_bytes(bytes).map_err(|e| PolicyError::Plist(e.to_string()))?;
    let dict = value
        .into_dictionary()
        .ok_or_else(|| PolicyError::Plist("top-level value is not a dictionary".to_string()))?;

    let policy_version = dict
        .get("policy_version")
        .and_then(|v| v.as_unsigned_integer())
        .map(|n| n as u32)
        .unwrap_or(1);
    let issued_by = dict
        .get("issued_by")
        .and_then(|v| v.as_string())
        .unwrap_or("")
        .to_string();
    let issued_at = dict
        .get("issued_at")
        .and_then(|v| v.as_string())
        .unwrap_or("")
        .to_string();
    let ui_banner = dict
        .get("ui_banner")
        .and_then(|v| v.as_string())
        .map(|s| s.to_string());

    let mut locked_keys: HashMap<String, String> = HashMap::new();
    if let Some(plist::Value::Dictionary(map)) = dict.get("locked_keys") {
        for (k, v) in map {
            // Coerce every value to a string so booleans + integers from the
            // plist round-trip into the Config::get -> str layer cleanly.
            let s = match v {
                plist::Value::String(s) => s.clone(),
                plist::Value::Boolean(b) => b.to_string(),
                plist::Value::Integer(i) => i.to_string(),
                plist::Value::Real(f) => f.to_string(),
                other => format!("{other:?}"),
            };
            locked_keys.insert(k.clone(), s);
        }
    }

    Ok(EnterprisePolicy {
        policy_version,
        issued_by,
        issued_at,
        locked_keys,
        ui_banner,
    })
}

/// Stub on non-macOS so callers don't need cfg gates.
#[cfg(not(target_os = "macos"))]
pub fn parse_plist(_bytes: &[u8]) -> Result<EnterprisePolicy, PolicyError> {
    Err(PolicyError::Plist(
        "plist parsing not available off macOS".to_string(),
    ))
}

/// Load the JSON policy file at `path`. `Ok(None)` if the file is missing —
/// that is the normal unmanaged case, not an error.
pub fn load_json_from(path: &Path) -> Result<Option<EnterprisePolicy>, PolicyError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(parse_json(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PolicyError::Io(e)),
    }
}

/// Load the Managed Preferences plist at `path` (macOS only in practice).
pub fn load_plist_from(path: &Path) -> Result<Option<EnterprisePolicy>, PolicyError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(parse_plist(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PolicyError::Io(e)),
    }
}

/// Load + merge from explicit paths. The platform-specific entry points
/// (`thane-platform`'s `MacosDirs::policy_file_path()`, etc.) provide the
/// real paths; we accept them here so this module stays platform-agnostic
/// and unit-testable without touching the real filesystem.
pub fn load_from_paths(
    json_path: &Path,
    plist_path: Option<&Path>,
) -> Result<Option<EnterprisePolicy>, PolicyError> {
    let json = load_json_from(json_path)?;
    let plist = match plist_path {
        Some(p) => load_plist_from(p)?,
        None => None,
    };
    Ok(merge_policies(json, plist))
}

/// Best-effort full-platform load. Returns `Ok(None)` on the common no-MDM
/// case. Errors are returned so the caller can decide whether to emit an
/// audit event and continue.
// `return`s are structurally required so each cfg-gated block can be the
// function's effective tail; without them the non-matching branches would
// fall through to the no-op at the end. clippy's needless_return lint
// doesn't account for cfg-driven control flow.
#[allow(clippy::needless_return)]
pub fn load_for_platform() -> Result<Option<EnterprisePolicy>, PolicyError> {
    // Path strings live here rather than reaching into `thane-platform`,
    // because `thane-core` may not depend on it (cycle).
    #[cfg(target_os = "macos")]
    {
        let json_path = PathBuf::from("/Library/Application Support/thane/policy.json");
        let plist_path = PathBuf::from("/Library/Managed Preferences/com.thane.app.plist");
        return load_from_paths(&json_path, Some(&plist_path));
    }
    #[cfg(target_os = "linux")]
    {
        let json_path = PathBuf::from("/etc/thane/policy.json");
        return load_from_paths(&json_path, None);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_tmp(contents: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("tmpfile");
        f.write_all(contents).expect("write");
        f
    }

    #[test]
    fn parse_well_formed_json() {
        let raw = br#"{
            "policy_version": 1,
            "issued_by": "Acme Corp IT Security",
            "issued_at": "2026-05-25T00:00:00Z",
            "locked_keys": {
                "audit-sink-loki-enabled": "true",
                "audit-allow-clear": "false"
            },
            "ui_banner": "Enterprise audit policy active"
        }"#;
        let p = parse_json(raw).expect("parse");
        assert_eq!(p.policy_version, 1);
        assert_eq!(p.issued_by, "Acme Corp IT Security");
        assert_eq!(p.lookup("audit-sink-loki-enabled"), Some("true"));
        assert!(p.is_locked("audit-allow-clear"));
        assert!(!p.is_locked("font-size"));
        assert_eq!(p.ui_banner.as_deref(), Some("Enterprise audit policy active"));
    }

    #[test]
    fn load_json_returns_none_when_missing() {
        let path = std::path::Path::new("/definitely/does/not/exist/policy.json");
        let p = load_json_from(path).expect("must not error on ENOENT");
        assert!(p.is_none());
    }

    #[test]
    fn load_json_returns_err_on_bad_syntax() {
        let f = write_tmp(b"{this is not valid json");
        let err = load_json_from(f.path()).expect_err("must error");
        assert!(matches!(err, PolicyError::Json(_)));
    }

    #[test]
    fn merge_prefers_plist_locked_keys_on_conflict() {
        let json = EnterprisePolicy {
            policy_version: 1,
            issued_by: "json-issuer".into(),
            issued_at: "2026-05-01".into(),
            locked_keys: HashMap::from([
                ("audit-sink-loki-enabled".to_string(), "false".to_string()),
                ("audit-sink-loki-url".to_string(), "https://json.example".to_string()),
            ]),
            ui_banner: Some("json banner".into()),
        };
        let plist = EnterprisePolicy {
            policy_version: 2,
            issued_by: "plist-issuer".into(),
            issued_at: "2026-05-25".into(),
            locked_keys: HashMap::from([
                // overrides json
                ("audit-sink-loki-enabled".to_string(), "true".to_string()),
                // unique to plist
                ("audit-redaction-policy".to_string(), "strict".to_string()),
            ]),
            ui_banner: Some("plist banner".into()),
        };
        let merged = merge_policies(Some(json), Some(plist)).expect("merged");
        // Plist won the conflict
        assert_eq!(merged.lookup("audit-sink-loki-enabled"), Some("true"));
        // Json's unique key still present
        assert_eq!(
            merged.lookup("audit-sink-loki-url"),
            Some("https://json.example")
        );
        // Plist unique key present
        assert_eq!(merged.lookup("audit-redaction-policy"), Some("strict"));
        // Plist took over issued_by + version + banner
        assert_eq!(merged.issued_by, "plist-issuer");
        assert_eq!(merged.policy_version, 2);
        assert_eq!(merged.ui_banner.as_deref(), Some("plist banner"));
    }

    #[test]
    fn merge_with_only_one_source_returns_that_source() {
        let p = EnterprisePolicy {
            policy_version: 1,
            issued_by: "x".into(),
            issued_at: "y".into(),
            locked_keys: HashMap::new(),
            ui_banner: None,
        };
        assert_eq!(merge_policies(Some(p.clone()), None).unwrap(), p);
        assert_eq!(merge_policies(None, Some(p.clone())).unwrap(), p);
        assert!(merge_policies(None, None).is_none());
    }

    #[test]
    fn load_from_paths_returns_none_when_both_absent() {
        let json = std::path::Path::new("/nope/policy.json");
        let plist = std::path::Path::new("/nope/managed.plist");
        let res = load_from_paths(json, Some(plist)).expect("ENOENT must not error");
        assert!(res.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_plist_round_trips_string_locked_keys() {
        // Build a minimal plist with the expected shape.
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>policy_version</key>
    <integer>1</integer>
    <key>issued_by</key>
    <string>Acme MDM</string>
    <key>issued_at</key>
    <string>2026-05-25T00:00:00Z</string>
    <key>locked_keys</key>
    <dict>
        <key>audit-sink-loki-enabled</key>
        <string>true</string>
        <key>audit-allow-clear</key>
        <false/>
    </dict>
    <key>ui_banner</key>
    <string>Acme audit policy</string>
</dict>
</plist>"#;
        let p = parse_plist(xml).expect("parse plist");
        assert_eq!(p.issued_by, "Acme MDM");
        // String value passed through unchanged.
        assert_eq!(p.lookup("audit-sink-loki-enabled"), Some("true"));
        // Boolean false coerced into the string "false" so Config::get_parsed
        // can interpret it just like a user config value.
        assert_eq!(p.lookup("audit-allow-clear"), Some("false"));
        assert_eq!(p.ui_banner.as_deref(), Some("Acme audit policy"));
    }

    #[test]
    fn missing_optional_fields_get_sensible_defaults() {
        // Only `locked_keys` required in practice.
        let raw = br#"{ "locked_keys": { "audit-sink-loki-enabled": "true" } }"#;
        let p = parse_json(raw).expect("parse");
        assert_eq!(p.policy_version, 1);
        assert_eq!(p.issued_by, "");
        assert_eq!(p.issued_at, "");
        assert!(p.ui_banner.is_none());
        assert!(p.is_locked("audit-sink-loki-enabled"));
    }
}
