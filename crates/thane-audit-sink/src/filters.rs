//! Helpers for parsing per-sink filter config strings.

use std::collections::HashSet;

use thane_core::audit::AuditSeverity;

use crate::AuditEventTypeKey;

/// Parse a min-severity config string into an [`AuditSeverity`].
///
/// Accepts the same lowercase strings the rest of the codebase uses; falls
/// back to `Info` on anything unknown (matching the audit_redaction policy
/// pattern — a typo never silently disables the filter).
pub fn parse_min_severity(raw: &str) -> AuditSeverity {
    match raw.trim().to_ascii_lowercase().as_str() {
        "warning" | "warn" => AuditSeverity::Warning,
        "alert" => AuditSeverity::Alert,
        "critical" | "crit" => AuditSeverity::Critical,
        _ => AuditSeverity::Info,
    }
}

/// Parse a comma-separated list of event type keys (snake_case) into a set.
/// Empty / whitespace-only string yields `None` (accept all).
pub fn parse_event_filter(raw: &str) -> Option<HashSet<AuditEventTypeKey>> {
    let set: HashSet<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if set.is_empty() { None } else { Some(set) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_severities() {
        assert_eq!(parse_min_severity("info"), AuditSeverity::Info);
        assert_eq!(parse_min_severity("warning"), AuditSeverity::Warning);
        assert_eq!(parse_min_severity("warn"), AuditSeverity::Warning);
        assert_eq!(parse_min_severity("Alert"), AuditSeverity::Alert);
        assert_eq!(parse_min_severity("critical"), AuditSeverity::Critical);
        assert_eq!(parse_min_severity("garbage"), AuditSeverity::Info);
    }

    #[test]
    fn empty_event_filter_means_accept_all() {
        assert!(parse_event_filter("").is_none());
        assert!(parse_event_filter("  ,, ").is_none());
    }

    #[test]
    fn event_filter_parses_csv() {
        let f = parse_event_filter("secret_access, file_write").unwrap();
        assert!(f.contains("secret_access"));
        assert!(f.contains("file_write"));
        assert_eq!(f.len(), 2);
    }
}
