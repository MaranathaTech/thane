use serde::Deserialize;
use std::path::PathBuf;

/// A JSONL conversation record from Claude Code's session files.
#[derive(Debug, Deserialize)]
struct ConversationRecord {
    #[serde(default)]
    uuid: String,
    #[serde(default, rename = "type")]
    record_type: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default, rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    message: Option<MessageRecord>,
}

#[derive(Debug, Deserialize)]
struct MessageRecord {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: Option<MessageContent>,
}

/// Claude Code stores message content as either a plain string
/// or an array of content blocks (e.g. `[{"type":"text","text":"..."}]`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(default, rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
}

impl MessageContent {
    fn as_text(&self) -> Option<String> {
        match self {
            MessageContent::Text(s) => {
                if s.is_empty() {
                    None
                } else {
                    Some(s.clone())
                }
            }
            MessageContent::Blocks(blocks) => {
                let texts: Vec<&str> = blocks
                    .iter()
                    .filter(|b| b.block_type == "text")
                    .filter_map(|b| b.text.as_deref())
                    .collect();
                if texts.is_empty() {
                    None
                } else {
                    Some(texts.join("\n"))
                }
            }
        }
    }
}

/// A user prompt extracted from a Claude Code session JSONL file.
#[derive(Debug, Clone)]
pub struct PromptRecord {
    pub uuid: String,
    pub timestamp: String,
    pub session_id: String,
    pub text: String,
}

/// Scan Claude Code project JSONL files for user prompts.
/// Uses the same path-mangling as `CostTracker::for_project()`:
/// CWD `/foo/bar` → directory name `foo-bar` under `~/.claude/projects/`.
pub fn scan_project_prompts(cwd: &str) -> Vec<PromptRecord> {
    let claude_dir = default_claude_dir();
    let projects_dir = claude_dir.join("projects");
    if !projects_dir.is_dir() {
        return Vec::new();
    }

    let mangled = cwd.replace('/', "-");
    let project_dir = projects_dir.join(&mangled);
    if !project_dir.is_dir() {
        return Vec::new();
    }

    let mut prompts = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&project_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                parse_prompts_from_file(&path, &mut prompts);
            }
        }
    }
    prompts
}

/// Check if text looks like an actual human-typed prompt rather than
/// system-injected content (task notifications, command output, system reminders, etc.).
fn is_human_prompt(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    // System-injected content starts with XML-like tags.
    if trimmed.starts_with('<') {
        return false;
    }
    true
}

fn parse_prompts_from_file(path: &std::path::Path, prompts: &mut Vec<PromptRecord>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let record: ConversationRecord = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(_) => continue,
        };

        // Only user messages are prompts.
        if record.record_type != "user" {
            continue;
        }
        let message = match &record.message {
            Some(m) if m.role == "user" => m,
            _ => continue,
        };
        let text = match message.content.as_ref().and_then(|c| c.as_text()) {
            Some(t) if is_human_prompt(&t) => t,
            _ => continue,
        };

        prompts.push(PromptRecord {
            uuid: record.uuid,
            timestamp: record.timestamp,
            session_id: record.session_id,
            text,
        });
    }
}

/// Get the default Claude Code directory (~/.claude).
fn default_claude_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".claude")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_content_text() {
        let content = MessageContent::Text("hello".to_string());
        assert_eq!(content.as_text(), Some("hello".to_string()));
    }

    #[test]
    fn test_message_content_empty_text() {
        let content = MessageContent::Text(String::new());
        assert_eq!(content.as_text(), None);
    }

    #[test]
    fn test_message_content_blocks() {
        let content = MessageContent::Blocks(vec![
            ContentBlock {
                block_type: "text".to_string(),
                text: Some("first".to_string()),
            },
            ContentBlock {
                block_type: "image".to_string(),
                text: None,
            },
            ContentBlock {
                block_type: "text".to_string(),
                text: Some("second".to_string()),
            },
        ]);
        assert_eq!(content.as_text(), Some("first\nsecond".to_string()));
    }

    #[test]
    fn test_message_content_empty_blocks() {
        let content = MessageContent::Blocks(vec![]);
        assert_eq!(content.as_text(), None);
    }

    #[test]
    fn test_parse_user_record() {
        let line = r#"{"uuid":"abc-123","type":"user","timestamp":"2025-01-01T00:00:00Z","sessionId":"sess-1","message":{"role":"user","content":"fix the bug"}}"#;
        let record: ConversationRecord = serde_json::from_str(line).unwrap();
        assert_eq!(record.record_type, "user");
        assert_eq!(record.uuid, "abc-123");
        let msg = record.message.unwrap();
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content.unwrap().as_text(), Some("fix the bug".to_string()));
    }

    #[test]
    fn test_parse_assistant_record_ignored() {
        let line = r#"{"uuid":"def-456","type":"assistant","timestamp":"2025-01-01T00:00:01Z","sessionId":"sess-1","message":{"role":"assistant","content":"I'll fix that."}}"#;
        let record: ConversationRecord = serde_json::from_str(line).unwrap();
        assert_eq!(record.record_type, "assistant");
    }

    #[test]
    fn test_parse_block_content() {
        let line = r#"{"uuid":"ghi-789","type":"user","timestamp":"2025-01-01T00:00:02Z","sessionId":"sess-1","message":{"role":"user","content":[{"type":"text","text":"hello world"}]}}"#;
        let record: ConversationRecord = serde_json::from_str(line).unwrap();
        let msg = record.message.unwrap();
        assert_eq!(msg.content.unwrap().as_text(), Some("hello world".to_string()));
    }

    #[test]
    fn test_scan_nonexistent_project() {
        let prompts = scan_project_prompts("/nonexistent/path/that/does/not/exist/zzzzz");
        assert!(prompts.is_empty());
    }

    #[test]
    fn test_is_human_prompt() {
        assert!(is_human_prompt("Fix the bug in main.rs"));
        assert!(is_human_prompt("please kill app and relaunch"));
        assert!(!is_human_prompt(""));
        assert!(!is_human_prompt("   "));
        assert!(!is_human_prompt("<task-notification><task-id>abc</task-id></task-notification>"));
        assert!(!is_human_prompt("<system-reminder>reminder text</system-reminder>"));
        assert!(!is_human_prompt("<local-command-caveat>caveat</local-command-caveat>"));
        assert!(!is_human_prompt("<command-name>/clear</command-name>"));
    }
}
