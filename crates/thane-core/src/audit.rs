use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use uuid::Uuid;

/// An audit event recording agent activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub workspace_id: Uuid,
    pub panel_id: Option<Uuid>,
    pub event_type: AuditEventType,
    pub severity: AuditSeverity,
    /// Human-readable description of the event.
    pub description: String,
    /// Raw data associated with the event (e.g. the command, file path, etc.).
    pub metadata: serde_json::Value,
    /// The name of the agent that generated this event (e.g. "claude", "codex"),
    /// if attributable to a specific agent running in the panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// SHA-256 hash of the previous event's serialized form (empty for the first event).
    #[serde(default)]
    pub prev_hash: String,
}

/// Types of auditable events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    /// Agent executed a shell command.
    CommandExecuted,
    /// Agent read a file.
    FileRead,
    /// Agent wrote or modified a file.
    FileWrite,
    /// Agent deleted a file.
    FileDelete,
    /// Agent accessed a secret/sensitive file.
    SecretAccess,
    /// Agent accessed a private key file.
    PrivateKeyAccess,
    /// Potential PII detected in agent input/output.
    PiiDetected,
    /// Agent made a network request.
    NetworkAccess,
    /// Agent spawned a subprocess.
    ProcessSpawn,
    /// Agent accessed environment variables (which may contain secrets).
    EnvVarAccess,
    /// Agent used the browser to navigate.
    BrowserNavigation,
    /// Agent executed JavaScript in the browser.
    BrowserJsExecution,
    /// Agent used the socket API.
    RpcCall,
    /// Sandbox was enabled or disabled for a workspace.
    SandboxToggle,
    /// Sandbox violation: agent tried to access a denied path.
    SandboxViolation,
    /// Sandbox policy was modified (path added/removed).
    SandboxPolicyChange,
    /// Agent (Claude Code, etc.) was invoked from the terminal.
    AgentInvocation,
    /// User prompt sent to Claude Code during an interactive session.
    UserPrompt,
    /// A task was added to the agent queue.
    QueueTaskSubmitted,
    /// A queued task's headless process was spawned.
    QueueTaskStarted,
    /// A queued task completed successfully.
    QueueTaskCompleted,
    /// A queued task failed (spawn error, non-zero exit, poll error).
    QueueTaskFailed,
    /// A queued task was cancelled by the user.
    QueueTaskCancelled,
    /// The model selected by Claude Code for a queue task.
    QueueModelSelected,
    /// Custom event from an integration.
    Custom(String),
}

/// Severity levels for audit events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    /// Informational — normal agent activity.
    Info,
    /// Warning — potentially sensitive operation.
    Warning,
    /// Alert — high-sensitivity operation (secret access, PII).
    Alert,
    /// Critical — operation that requires immediate attention.
    Critical,
}

/// Configurable action when a sensitive operation is detected.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveOpAction {
    /// Allow the operation, only log it (no notification).
    Allow,
    /// Log and send a warning notification, but don't interrupt.
    #[default]
    Warn,
    /// Log, notify, and send SIGTSTP to the terminal's child process to pause it.
    Block,
}

/// Patterns for detecting sensitive file access.
pub const SENSITIVE_FILE_PATTERNS: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    ".env.secret",
    "credentials",
    "credentials.json",
    "secrets",
    "secrets.yaml",
    "secrets.yml",
    "secrets.json",
    ".aws/credentials",
    ".ssh/id_rsa",
    ".ssh/id_ed25519",
    ".ssh/id_ecdsa",
    ".ssh/id_dsa",
    ".ssh/config",
    ".gnupg/",
    ".pgpass",
    ".netrc",
    "service-account.json",
    "keystore",
    ".p12",
    ".pfx",
    ".pem",
    ".key",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "master.key",
    "encryption.key",
];

/// Patterns that may indicate PII in text.
pub const PII_PATTERNS: &[&str] = &[
    // Email-like patterns are handled by regex in the detector
    "social security",
    "SSN",
    "date of birth",
    "passport",
    "driver's license",
    "credit card",
    "bank account",
    "routing number",
];

/// Check if a file path matches known sensitive patterns.
pub fn is_sensitive_file(path: &str) -> Option<AuditEventType> {
    let path_lower = path.to_lowercase();

    // Check for private key files.
    if path_lower.ends_with(".pem")
        || path_lower.ends_with(".key")
        || path_lower.ends_with(".p12")
        || path_lower.ends_with(".pfx")
        || path_lower.contains(".ssh/id_")
    {
        return Some(AuditEventType::PrivateKeyAccess);
    }

    // Check for other sensitive files.
    for pattern in SENSITIVE_FILE_PATTERNS {
        if path_lower.contains(&pattern.to_lowercase()) {
            return Some(AuditEventType::SecretAccess);
        }
    }

    None
}

/// Check text content for potential PII patterns.
///
/// Returns a list of detected PII types.
pub fn detect_pii(text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    let text_lower = text.to_lowercase();

    // Check for PII keywords.
    for pattern in PII_PATTERNS {
        if text_lower.contains(&pattern.to_lowercase()) {
            findings.push(format!("Keyword match: {pattern}"));
        }
    }

    // Simple email pattern detection (not regex, for speed).
    if text.contains('@') {
        let words: Vec<&str> = text.split_whitespace().collect();
        for word in words {
            if word.contains('@') && word.contains('.') && word.len() > 5 {
                let at_pos = word.find('@').unwrap();
                let local = &word[..at_pos];
                let domain = &word[at_pos + 1..];
                if local.len() >= 2 && domain.contains('.') && domain.len() >= 4 {
                    findings.push("Possible email address detected".to_string());
                    break;
                }
            }
        }
    }

    // Simple SSN pattern (XXX-XX-XXXX).
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len().saturating_sub(10) {
        if chars[i].is_ascii_digit()
            && chars[i + 1].is_ascii_digit()
            && chars[i + 2].is_ascii_digit()
            && chars[i + 3] == '-'
            && chars[i + 4].is_ascii_digit()
            && chars[i + 5].is_ascii_digit()
            && chars[i + 6] == '-'
            && chars[i + 7].is_ascii_digit()
            && chars[i + 8].is_ascii_digit()
            && chars[i + 9].is_ascii_digit()
            && chars[i + 10].is_ascii_digit()
        {
            findings.push("Possible SSN pattern (XXX-XX-XXXX) detected".to_string());
            break;
        }
    }

    findings
}

/// Extract file-path-like strings from raw terminal output text.
/// Looks for absolute paths (/...) and home-relative paths (~/).
/// Returns a list of candidate paths found in the text.
pub fn extract_file_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();

    for word in text.split(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == '`') {
        let trimmed = word.trim_matches(|c: char| c == ',' || c == ';' || c == ':' || c == ')' || c == '(');
        if trimmed.len() < 3 {
            continue;
        }
        // Absolute path or home-relative path.
        if trimmed.starts_with('/') || trimmed.starts_with("~/") {
            // Must contain at least one path separator after the first character.
            if trimmed[1..].contains('/') || trimmed.contains('.') {
                paths.push(trimmed.to_string());
            }
        }
    }

    paths
}

/// Strip ANSI escape sequences and other terminal control codes from text.
///
/// Removes CSI sequences (ESC [ ... final_byte), OSC sequences (ESC ] ... ST),
/// simple ESC-letter sequences, and lone control characters (except newline/tab).
/// This produces clean text suitable for audit log display.
pub fn strip_terminal_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        if ch == '\x1b' && i + 1 < len {
            match chars[i + 1] {
                // CSI: ESC [ ... (params) final byte (0x40–0x7E)
                '[' => {
                    i += 2;
                    while i < len && !(chars[i] as u32 >= 0x40 && chars[i] as u32 <= 0x7E) {
                        i += 1;
                    }
                    if i < len { i += 1; } // skip final byte
                    continue;
                }
                // OSC: ESC ] ... (terminated by BEL or ST)
                ']' => {
                    i += 2;
                    while i < len {
                        if chars[i] == '\x07' { i += 1; break; }             // BEL
                        if chars[i] == '\x1b' && i + 1 < len && chars[i + 1] == '\\' {
                            i += 2; break;                                     // ST
                        }
                        i += 1;
                    }
                    continue;
                }
                // Simple ESC + letter (e.g., ESC M, ESC 7, ESC 8)
                c if c.is_ascii_alphabetic() || c.is_ascii_digit() => {
                    i += 2;
                    continue;
                }
                _ => {
                    i += 2;
                    continue;
                }
            }
        }

        // Strip lone control characters (but keep \n, \t, \r)
        if ch.is_control() && ch != '\n' && ch != '\t' && ch != '\r' {
            i += 1;
            continue;
        }

        out.push(ch);
        i += 1;
    }

    out
}

/// Results from scanning a queue task's output log for sensitive data.
#[derive(Debug, Clone, Default)]
pub struct QueueOutputScanResult {
    /// Sensitive file paths found and their event types.
    pub sensitive_files: Vec<(String, AuditEventType)>,
    /// PII findings (human-readable descriptions).
    pub pii_findings: Vec<String>,
}

/// Scan a queue task's output log file for sensitive file references and PII.
///
/// Reuses [`extract_file_paths`], [`is_sensitive_file`], and [`detect_pii`].
/// Returns an empty result on read error (missing file, permissions, etc.).
pub fn scan_queue_output_log(log_path: &str) -> QueueOutputScanResult {
    let text = match std::fs::read_to_string(log_path) {
        Ok(t) => t,
        Err(_) => return QueueOutputScanResult::default(),
    };

    let mut result = QueueOutputScanResult::default();

    for path in extract_file_paths(&text) {
        if let Some(event_type) = is_sensitive_file(&path) {
            result.sensitive_files.push((path, event_type));
        }
    }

    result.pii_findings = detect_pii(&text);

    result
}

/// Audit log storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditLog {
    events: Vec<AuditEvent>,
    max_events: usize,
}

impl AuditLog {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Vec::new(),
            max_events,
        }
    }

    /// Record an audit event.
    pub fn record(&mut self, event: AuditEvent) {
        self.events.push(event);
        if self.events.len() > self.max_events {
            self.events.remove(0);
        }
    }

    /// Compute the SHA-256 hash of the previous event's serialized form.
    fn prev_event_hash(&self) -> String {
        match self.events.last() {
            Some(event) => {
                let serialized = serde_json::to_string(event).unwrap_or_default();
                let mut hasher = Sha256::new();
                hasher.update(serialized.as_bytes());
                format!("{:x}", hasher.finalize())
            }
            None => String::new(),
        }
    }

    /// Create and record a new audit event.
    pub fn log(
        &mut self,
        workspace_id: Uuid,
        panel_id: Option<Uuid>,
        event_type: AuditEventType,
        severity: AuditSeverity,
        description: impl Into<String>,
        metadata: serde_json::Value,
    ) {
        self.log_with_agent(workspace_id, panel_id, event_type, severity, description, metadata, None);
    }

    /// Create and record a new audit event attributed to a specific agent.
    pub fn log_with_agent(
        &mut self,
        workspace_id: Uuid,
        panel_id: Option<Uuid>,
        event_type: AuditEventType,
        severity: AuditSeverity,
        description: impl Into<String>,
        metadata: serde_json::Value,
        agent_name: Option<String>,
    ) {
        let prev_hash = self.prev_event_hash();
        let event = AuditEvent {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            workspace_id,
            panel_id,
            event_type,
            severity,
            description: description.into(),
            metadata,
            agent_name,
            prev_hash,
        };
        self.record(event);
    }

    /// Get all events.
    pub fn all(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Get events filtered by severity.
    pub fn by_severity(&self, min_severity: AuditSeverity) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.severity >= min_severity)
            .collect()
    }

    /// Get events filtered by workspace.
    pub fn by_workspace(&self, workspace_id: Uuid) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| e.workspace_id == workspace_id)
            .collect()
    }

    /// Get alerts (warnings and above).
    pub fn alerts(&self) -> Vec<&AuditEvent> {
        self.by_severity(AuditSeverity::Warning)
    }

    /// Get secret/PII access events.
    pub fn sensitive_access_events(&self) -> Vec<&AuditEvent> {
        self.events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    AuditEventType::SecretAccess
                        | AuditEventType::PrivateKeyAccess
                        | AuditEventType::PiiDetected
                )
            })
            .collect()
    }

    /// Get event count.
    pub fn count(&self) -> usize {
        self.events.len()
    }

    /// Export audit log as JSON (for enterprise dashboard).
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.events)
    }

    /// Clear all events, retaining a Critical "AuditCleared" record.
    pub fn clear(&mut self) {
        let count = self.events.len();
        self.log(
            Uuid::nil(),
            None,
            AuditEventType::Custom("AuditCleared".to_string()),
            AuditSeverity::Critical,
            format!("Audit log cleared ({count} events removed)"),
            serde_json::json!({"events_cleared": count}),
        );
        // Keep only the clear marker (last event)
        if let Some(clear_event) = self.events.last().cloned() {
            self.events = vec![clear_event];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sensitive_file_detection() {
        assert_eq!(
            is_sensitive_file("/home/user/.env"),
            Some(AuditEventType::SecretAccess)
        );
        assert_eq!(
            is_sensitive_file("/home/user/.ssh/id_rsa"),
            Some(AuditEventType::PrivateKeyAccess)
        );
        assert_eq!(
            is_sensitive_file("/home/user/project/server.key"),
            Some(AuditEventType::PrivateKeyAccess)
        );
        assert_eq!(
            is_sensitive_file("/home/user/project/credentials.json"),
            Some(AuditEventType::SecretAccess)
        );
        assert_eq!(is_sensitive_file("/home/user/project/main.rs"), None);
    }

    #[test]
    fn test_pii_detection() {
        let findings = detect_pii("My email is user@example.com");
        assert!(!findings.is_empty());

        let findings = detect_pii("SSN: 123-45-6789");
        assert!(!findings.is_empty());

        let findings = detect_pii("Hello world, nothing sensitive here");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_audit_log() {
        let mut log = AuditLog::new(100);
        let ws_id = Uuid::new_v4();

        log.log(
            ws_id,
            None,
            AuditEventType::CommandExecuted,
            AuditSeverity::Info,
            "Executed: ls -la",
            serde_json::json!({"command": "ls -la"}),
        );

        log.log(
            ws_id,
            None,
            AuditEventType::SecretAccess,
            AuditSeverity::Alert,
            "Agent accessed .env file",
            serde_json::json!({"path": "/project/.env"}),
        );

        assert_eq!(log.count(), 2);
        assert_eq!(log.alerts().len(), 1);
        assert_eq!(log.sensitive_access_events().len(), 1);
    }

    #[test]
    fn test_audit_log_max_events() {
        let mut log = AuditLog::new(2);
        let ws_id = Uuid::new_v4();

        for i in 0..5 {
            log.log(
                ws_id,
                None,
                AuditEventType::CommandExecuted,
                AuditSeverity::Info,
                format!("Event {i}"),
                serde_json::json!({}),
            );
        }

        assert_eq!(log.count(), 2);
        assert_eq!(log.all()[0].description, "Event 3");
    }

    #[test]
    fn test_extract_file_paths_absolute() {
        let text = "cat /home/user/.env and read /etc/passwd";
        let paths = extract_file_paths(text);
        assert!(paths.contains(&"/home/user/.env".to_string()));
        assert!(paths.contains(&"/etc/passwd".to_string()));
    }

    #[test]
    fn test_extract_file_paths_home_relative() {
        let text = "reading ~/.ssh/id_rsa for keys";
        let paths = extract_file_paths(text);
        assert!(paths.contains(&"~/.ssh/id_rsa".to_string()));
    }

    #[test]
    fn test_extract_file_paths_quoted() {
        let text = "opened '/home/user/credentials.json' for reading";
        let paths = extract_file_paths(text);
        assert!(paths.contains(&"/home/user/credentials.json".to_string()));
    }

    #[test]
    fn test_extract_file_paths_no_paths() {
        let text = "Hello world, nothing here";
        let paths = extract_file_paths(text);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_extract_file_paths_backtick_quoted() {
        let text = "reading `/home/user/.env` for config";
        let paths = extract_file_paths(text);
        assert!(paths.contains(&"/home/user/.env".to_string()));
    }

    #[test]
    fn test_extract_file_paths_with_trailing_colon() {
        let text = "error in /home/user/src/main.rs: line 42";
        let paths = extract_file_paths(text);
        assert!(paths.contains(&"/home/user/src/main.rs".to_string()));
    }

    #[test]
    fn test_extract_file_paths_short_paths_ignored() {
        // Paths shorter than 3 chars should be ignored
        let text = "/ /a";
        let paths = extract_file_paths(text);
        assert!(paths.is_empty());
    }

    #[test]
    fn test_detect_pii_ssn_at_end_of_string() {
        let findings = detect_pii("my number is 123-45-6789");
        assert!(findings.iter().any(|f| f.contains("SSN")));
    }

    #[test]
    fn test_detect_pii_no_false_positive_short_email() {
        // "a@b.c" is too short (len <= 5) — should not match
        let findings = detect_pii("contact a@b.c for info");
        assert!(!findings.iter().any(|f| f.contains("email")));
    }

    #[test]
    fn test_detect_pii_ssn_not_enough_digits() {
        // Only 9 chars total (XXX-XX-XXX) — should NOT match SSN pattern
        let findings = detect_pii("number: 123-45-678");
        assert!(!findings.iter().any(|f| f.contains("SSN")));
    }

    #[test]
    fn test_is_sensitive_file_case_insensitive() {
        assert_eq!(
            is_sensitive_file("/home/user/.ENV"),
            Some(AuditEventType::SecretAccess)
        );
        assert_eq!(
            is_sensitive_file("/home/user/.Env.Local"),
            Some(AuditEventType::SecretAccess)
        );
        assert_eq!(
            is_sensitive_file("/project/SERVER.KEY"),
            Some(AuditEventType::PrivateKeyAccess)
        );
    }

    #[test]
    fn test_audit_log_by_workspace() {
        let mut log = AuditLog::new(100);
        let ws1 = Uuid::new_v4();
        let ws2 = Uuid::new_v4();

        log.log(ws1, None, AuditEventType::CommandExecuted, AuditSeverity::Info, "cmd1", serde_json::json!({}));
        log.log(ws2, None, AuditEventType::FileRead, AuditSeverity::Info, "read1", serde_json::json!({}));
        log.log(ws1, None, AuditEventType::FileWrite, AuditSeverity::Warning, "write1", serde_json::json!({}));

        let ws1_events = log.by_workspace(ws1);
        assert_eq!(ws1_events.len(), 2);
        let ws2_events = log.by_workspace(ws2);
        assert_eq!(ws2_events.len(), 1);
    }

    #[test]
    fn test_audit_log_export_json() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info, "test", serde_json::json!({}));

        let json = log.export_json().unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("command_executed"));
    }

    #[test]
    fn test_audit_log_clear() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info, "test", serde_json::json!({}));
        assert_eq!(log.count(), 1);

        log.clear();
        // clear() retains the Critical "AuditCleared" marker event
        assert_eq!(log.count(), 1);
        assert_eq!(log.all()[0].severity, AuditSeverity::Critical);
        assert_eq!(log.all()[0].event_type, AuditEventType::Custom("AuditCleared".to_string()));
    }

    #[test]
    fn test_audit_log_hash_chain() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info, "first", serde_json::json!({}));
        assert!(log.all()[0].prev_hash.is_empty(), "first event should have empty prev_hash");

        log.log(ws, None, AuditEventType::FileRead, AuditSeverity::Info, "second", serde_json::json!({}));
        assert!(!log.all()[1].prev_hash.is_empty(), "second event should have non-empty prev_hash");
        assert_eq!(log.all()[1].prev_hash.len(), 64, "SHA-256 hex should be 64 chars");
    }

    #[test]
    fn test_scan_queue_output_log_sensitive_files() {
        let dir = std::env::temp_dir().join("thane_test_scan_sensitive");
        let _ = std::fs::create_dir_all(&dir);
        let log_file = dir.join("output.log");
        std::fs::write(&log_file, "Reading /home/user/.env for config\nAlso accessed ~/.ssh/id_rsa\n").unwrap();

        let result = scan_queue_output_log(&log_file.to_string_lossy());
        assert!(result.sensitive_files.iter().any(|(p, t)| p.contains(".env") && *t == AuditEventType::SecretAccess));
        assert!(result.sensitive_files.iter().any(|(p, t)| p.contains(".ssh/id_rsa") && *t == AuditEventType::PrivateKeyAccess));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_scan_queue_output_log_pii() {
        let dir = std::env::temp_dir().join("thane_test_scan_pii");
        let _ = std::fs::create_dir_all(&dir);
        let log_file = dir.join("output.log");
        std::fs::write(&log_file, "Contact user@example.com for details\nSSN: 123-45-6789\n").unwrap();

        let result = scan_queue_output_log(&log_file.to_string_lossy());
        assert!(!result.pii_findings.is_empty());
        assert!(result.pii_findings.iter().any(|f| f.contains("email")));
        assert!(result.pii_findings.iter().any(|f| f.contains("SSN")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_scan_queue_output_log_missing_file() {
        let result = scan_queue_output_log("/nonexistent/path/output.log");
        assert!(result.sensitive_files.is_empty());
        assert!(result.pii_findings.is_empty());
    }

    #[test]
    fn test_strip_terminal_codes_csi() {
        // Color codes: ESC[31m ... ESC[0m
        assert_eq!(strip_terminal_codes("\x1b[31mhello\x1b[0m"), "hello");
        // Cursor movement
        assert_eq!(strip_terminal_codes("\x1b[2Jcleared"), "cleared");
        // Bold
        assert_eq!(strip_terminal_codes("\x1b[1mbold\x1b[22m text"), "bold text");
    }

    #[test]
    fn test_strip_terminal_codes_osc() {
        // Window title: ESC ] 0 ; title BEL
        assert_eq!(strip_terminal_codes("\x1b]0;my title\x07prompt"), "prompt");
        // OSC terminated with ST
        assert_eq!(strip_terminal_codes("\x1b]2;title\x1b\\text"), "text");
    }

    #[test]
    fn test_strip_terminal_codes_preserves_normal_text() {
        assert_eq!(strip_terminal_codes("hello world"), "hello world");
        assert_eq!(strip_terminal_codes("line1\nline2\ttab"), "line1\nline2\ttab");
        assert_eq!(strip_terminal_codes(""), "");
    }

    #[test]
    fn test_strip_terminal_codes_control_chars() {
        // Bell, backspace, etc. should be stripped
        assert_eq!(strip_terminal_codes("hello\x07world"), "helloworld");
        assert_eq!(strip_terminal_codes("back\x08space"), "backspace");
    }

    #[test]
    fn test_strip_terminal_codes_complex() {
        // Simulated codex-style output with colors + cursor + unicode
        let input = "\x1b[32m✓\x1b[0m File \x1b[1m\x1b[36mmain.rs\x1b[0m updated";
        assert_eq!(strip_terminal_codes(input), "✓ File main.rs updated");
    }

    #[test]
    fn test_log_with_agent_sets_agent_name() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log_with_agent(
            ws,
            None,
            AuditEventType::AgentInvocation,
            AuditSeverity::Info,
            "codex invoked",
            serde_json::json!({}),
            Some("codex".to_string()),
        );

        assert_eq!(log.all()[0].agent_name, Some("codex".to_string()));
    }

    #[test]
    fn test_log_without_agent_has_none() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info, "test", serde_json::json!({}));

        assert_eq!(log.all()[0].agent_name, None);
    }

    #[test]
    fn test_agent_name_backward_compat_deserialize() {
        // Old format without agent_name field should deserialize with None
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "timestamp": "2026-01-01T00:00:00Z",
            "workspace_id": "00000000-0000-0000-0000-000000000000",
            "panel_id": null,
            "event_type": "command_executed",
            "severity": "info",
            "description": "test",
            "metadata": {},
            "prev_hash": ""
        }"#;
        let event: AuditEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.agent_name, None);
    }

    #[test]
    fn test_agent_name_skipped_when_none_in_serialization() {
        let event = AuditEvent {
            id: Uuid::nil(),
            timestamp: Utc::now(),
            workspace_id: Uuid::nil(),
            panel_id: None,
            event_type: AuditEventType::CommandExecuted,
            severity: AuditSeverity::Info,
            description: "test".to_string(),
            metadata: serde_json::json!({}),
            agent_name: None,
            prev_hash: String::new(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("agent_name"), "agent_name should be skipped when None");
    }

    #[test]
    fn test_agent_name_present_when_some_in_serialization() {
        let event = AuditEvent {
            id: Uuid::nil(),
            timestamp: Utc::now(),
            workspace_id: Uuid::nil(),
            panel_id: None,
            event_type: AuditEventType::AgentInvocation,
            severity: AuditSeverity::Info,
            description: "claude invoked".to_string(),
            metadata: serde_json::json!({}),
            agent_name: Some("claude".to_string()),
            prev_hash: String::new(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"agent_name\":\"claude\""));
    }

    #[test]
    fn test_multiple_agents_in_audit_log() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log_with_agent(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "claude ran ls", serde_json::json!({"command": "ls"}), Some("claude".to_string()));
        log.log_with_agent(ws, None, AuditEventType::FileWrite, AuditSeverity::Warning,
            "codex wrote main.rs", serde_json::json!({"path": "main.rs"}), Some("codex".to_string()));
        log.log_with_agent(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "claude ran cargo test", serde_json::json!({"command": "cargo test"}), Some("claude".to_string()));
        log.log(ws, None, AuditEventType::FileRead, AuditSeverity::Info,
            "unattributed read", serde_json::json!({}));

        assert_eq!(log.count(), 4);

        // Filter by agent name
        let claude_events: Vec<_> = log.all().iter()
            .filter(|e| e.agent_name.as_deref() == Some("claude"))
            .collect();
        assert_eq!(claude_events.len(), 2);

        let codex_events: Vec<_> = log.all().iter()
            .filter(|e| e.agent_name.as_deref() == Some("codex"))
            .collect();
        assert_eq!(codex_events.len(), 1);

        let unattributed: Vec<_> = log.all().iter()
            .filter(|e| e.agent_name.is_none())
            .collect();
        assert_eq!(unattributed.len(), 1);
    }

    #[test]
    fn test_by_severity_filtering() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "info event", serde_json::json!({}));
        log.log(ws, None, AuditEventType::FileWrite, AuditSeverity::Warning,
            "warning event", serde_json::json!({}));
        log.log(ws, None, AuditEventType::SecretAccess, AuditSeverity::Alert,
            "alert event", serde_json::json!({}));
        log.log(ws, None, AuditEventType::SandboxViolation, AuditSeverity::Critical,
            "critical event", serde_json::json!({}));

        assert_eq!(log.by_severity(AuditSeverity::Info).len(), 4);
        assert_eq!(log.by_severity(AuditSeverity::Warning).len(), 3);
        assert_eq!(log.by_severity(AuditSeverity::Alert).len(), 2);
        assert_eq!(log.by_severity(AuditSeverity::Critical).len(), 1);
    }

    #[test]
    fn test_hash_chain_integrity_verification() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "first", serde_json::json!({}));
        log.log(ws, None, AuditEventType::FileRead, AuditSeverity::Info,
            "second", serde_json::json!({}));
        log.log(ws, None, AuditEventType::FileWrite, AuditSeverity::Warning,
            "third", serde_json::json!({}));

        // Verify: each event's prev_hash matches the SHA-256 of the previous event
        let events = log.all();
        assert!(events[0].prev_hash.is_empty());

        for i in 1..events.len() {
            let prev_serialized = serde_json::to_string(&events[i - 1]).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(prev_serialized.as_bytes());
            let expected_hash = format!("{:x}", hasher.finalize());
            assert_eq!(events[i].prev_hash, expected_hash,
                "hash chain broken at event {i}");
        }
    }

    #[test]
    fn test_sensitive_access_events_with_agent_attribution() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log_with_agent(ws, None, AuditEventType::SecretAccess, AuditSeverity::Alert,
            "claude accessed .env", serde_json::json!({"path": ".env"}),
            Some("claude".to_string()));
        log.log_with_agent(ws, None, AuditEventType::PrivateKeyAccess, AuditSeverity::Alert,
            "codex accessed id_rsa", serde_json::json!({"path": "~/.ssh/id_rsa"}),
            Some("codex".to_string()));
        log.log_with_agent(ws, None, AuditEventType::PiiDetected, AuditSeverity::Alert,
            "aider output contained SSN", serde_json::json!({}),
            Some("aider".to_string()));
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "normal command", serde_json::json!({}));

        let sensitive = log.sensitive_access_events();
        assert_eq!(sensitive.len(), 3);

        // All sensitive events should have agent attribution
        assert_eq!(sensitive[0].agent_name.as_deref(), Some("claude"));
        assert_eq!(sensitive[1].agent_name.as_deref(), Some("codex"));
        assert_eq!(sensitive[2].agent_name.as_deref(), Some("aider"));
    }

    #[test]
    fn test_agent_name_survives_serialization_roundtrip() {
        let event = AuditEvent {
            id: Uuid::nil(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap().with_timezone(&Utc),
            workspace_id: Uuid::nil(),
            panel_id: Some(Uuid::nil()),
            event_type: AuditEventType::AgentInvocation,
            severity: AuditSeverity::Info,
            description: "claude invoked".to_string(),
            metadata: serde_json::json!({"prompt": "fix the bug"}),
            agent_name: Some("claude".to_string()),
            prev_hash: "abc123".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: AuditEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.agent_name, Some("claude".to_string()));
        assert_eq!(deserialized.description, "claude invoked");
        assert_eq!(deserialized.event_type, AuditEventType::AgentInvocation);
        assert_eq!(deserialized.prev_hash, "abc123");
    }

    #[test]
    fn test_agent_events_across_workspaces() {
        let mut log = AuditLog::new(100);
        let ws1 = Uuid::new_v4();
        let ws2 = Uuid::new_v4();

        log.log_with_agent(ws1, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "claude in ws1", serde_json::json!({}), Some("claude".to_string()));
        log.log_with_agent(ws2, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "codex in ws2", serde_json::json!({}), Some("codex".to_string()));
        log.log_with_agent(ws1, None, AuditEventType::FileWrite, AuditSeverity::Warning,
            "claude writes in ws1", serde_json::json!({}), Some("claude".to_string()));

        // Filter by workspace, then check agent attribution
        let ws1_events = log.by_workspace(ws1);
        assert_eq!(ws1_events.len(), 2);
        assert!(ws1_events.iter().all(|e| e.agent_name.as_deref() == Some("claude")));

        let ws2_events = log.by_workspace(ws2);
        assert_eq!(ws2_events.len(), 1);
        assert_eq!(ws2_events[0].agent_name.as_deref(), Some("codex"));
    }

    #[test]
    fn test_export_json_preserves_agent_names() {
        let mut log = AuditLog::new(100);
        let ws = Uuid::new_v4();

        log.log_with_agent(ws, None, AuditEventType::AgentInvocation, AuditSeverity::Info,
            "gemini invoked", serde_json::json!({}), Some("gemini".to_string()));
        log.log(ws, None, AuditEventType::CommandExecuted, AuditSeverity::Info,
            "no agent", serde_json::json!({}));

        let json = log.export_json().unwrap();
        let events: Vec<AuditEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].agent_name.as_deref(), Some("gemini"));
        assert_eq!(events[1].agent_name, None);
    }

    #[test]
    fn test_detect_pii_keywords_case_insensitive() {
        let findings = detect_pii("Please provide your SOCIAL SECURITY number");
        assert!(findings.iter().any(|f| f.contains("social security")));

        let findings = detect_pii("Enter your Credit Card details");
        assert!(findings.iter().any(|f| f.contains("credit card")));
    }

    #[test]
    fn test_detect_pii_multiple_findings() {
        let text = "SSN: 123-45-6789, email: user@example.com, passport number required";
        let findings = detect_pii(text);
        assert!(findings.iter().any(|f| f.contains("SSN")));
        assert!(findings.iter().any(|f| f.contains("email")));
        assert!(findings.iter().any(|f| f.contains("passport")));
    }

    #[test]
    fn test_is_sensitive_file_all_private_key_extensions() {
        for ext in &[".pem", ".key", ".p12", ".pfx"] {
            let path = format!("/project/server{ext}");
            assert_eq!(is_sensitive_file(&path), Some(AuditEventType::PrivateKeyAccess),
                "Expected PrivateKeyAccess for {ext}");
        }
    }

    #[test]
    fn test_is_sensitive_file_ssh_variants() {
        for key in &["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"] {
            let path = format!("/home/user/.ssh/{key}");
            assert_eq!(is_sensitive_file(&path), Some(AuditEventType::PrivateKeyAccess),
                "Expected PrivateKeyAccess for .ssh/{key}");
        }
    }

    #[test]
    fn test_strip_terminal_codes_mixed_with_agent_output() {
        // Simulates agent output with colors wrapping file paths
        let input = "\x1b[33mWarning:\x1b[0m reading \x1b[1m/home/user/.env\x1b[0m";
        let cleaned = strip_terminal_codes(input);
        assert_eq!(cleaned, "Warning: reading /home/user/.env");

        // Verify sensitive file detection works on cleaned output
        let paths = extract_file_paths(&cleaned);
        assert!(paths.contains(&"/home/user/.env".to_string()));
        assert_eq!(is_sensitive_file(&paths[0]), Some(AuditEventType::SecretAccess));
    }
}
