/// Policy limits for session persistence.
#[derive(Debug, Clone)]
pub struct PersistPolicy {
    /// Maximum scrollback lines to capture per terminal.
    pub max_scrollback_lines: usize,
    /// Maximum number of workspaces to persist.
    pub max_workspaces: usize,
    /// Auto-save interval in seconds.
    pub auto_save_interval_secs: u64,
    /// Maximum total snapshot size in bytes.
    pub max_snapshot_bytes: usize,
}

impl Default for PersistPolicy {
    fn default() -> Self {
        Self {
            max_scrollback_lines: 5000,
            max_workspaces: 50,
            auto_save_interval_secs: 8,
            max_snapshot_bytes: 50 * 1024 * 1024, // 50 MB
        }
    }
}

impl PersistPolicy {
    /// Truncate scrollback to the configured limit, ensuring we don't cut
    /// in the middle of an ANSI escape sequence.
    pub fn truncate_scrollback(&self, scrollback: &str) -> String {
        let lines: Vec<&str> = scrollback.lines().collect();
        if lines.len() <= self.max_scrollback_lines {
            return scrollback.to_string();
        }

        // Keep the last N lines
        let start = lines.len() - self.max_scrollback_lines;
        let truncated: String = lines[start..].join("\n");

        // Ensure we're not in the middle of an escape sequence.
        // If the truncated content starts with a partial ESC sequence, skip to the next line.
        sanitize_ansi_start(&truncated)
    }
}

/// If the string starts with what looks like a partial ANSI escape,
/// skip past it to the next newline.
fn sanitize_ansi_start(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }

    // Check if we're inside an escape sequence (no ESC byte before a CSI parameter)
    // A partial escape looks like: missing ESC but has `;`, `[`, or ends with `m`
    // This is a heuristic — we look for common incomplete CSI patterns.
    if !bytes.is_empty() && bytes[0] != b'\x1b' {
        // Check if first few chars look like CSI params (digits, ;, m)
        let first_line_end = s.find('\n').unwrap_or(s.len());
        let first_line = &s[..first_line_end];

        let looks_partial = first_line.len() < 20
            && first_line
                .chars()
                .all(|c| c.is_ascii_digit() || c == ';' || c == 'm' || c == '[');

        if looks_partial && first_line_end < s.len() {
            return s[first_line_end + 1..].to_string();
        }
    }

    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_scrollback() {
        let policy = PersistPolicy {
            max_scrollback_lines: 3,
            ..Default::default()
        };

        let scrollback = "line1\nline2\nline3\nline4\nline5";
        let truncated = policy.truncate_scrollback(scrollback);
        assert_eq!(truncated, "line3\nline4\nline5");
    }

    #[test]
    fn test_no_truncation_needed() {
        let policy = PersistPolicy {
            max_scrollback_lines: 10,
            ..Default::default()
        };

        let scrollback = "line1\nline2\nline3";
        let truncated = policy.truncate_scrollback(scrollback);
        assert_eq!(truncated, scrollback);
    }
}
