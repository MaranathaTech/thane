//! Queue execution engine — dequeues tasks and runs them via Claude Code.
//!
//! The executor is a state machine that:
//! 1. Polls the agent queue for the next runnable entry
//! 2. Spawns a Claude Code process with the task content as a prompt
//! 3. Monitors output for token limit errors
//! 4. Updates entry status on completion/failure
//!
//! The actual process spawning is delegated to a callback provided by the
//! UI layer (GTK), keeping this module platform-agnostic.

use std::path::PathBuf;

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::agent_queue::{AgentQueue, QueueTokenUsage};
use crate::sandbox::SandboxPolicy;

/// Token limit error patterns to detect in Claude Code output.
const TOKEN_LIMIT_PATTERNS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "Rate limit reached",
    "too many requests",
    "Too many requests",
    "token limit exceeded",
    "You've exceeded your usage limit",
    "usage limit",
    "429",
    "quota exceeded",
    "quota_exceeded",
    "Please try again in",
    "retry after",
];

/// Completion patterns indicating the agent finished successfully.
#[allow(dead_code)]
const COMPLETION_PATTERNS: &[&str] = &[
    "Task completed",
    "Done!",
    "All done",
    "finished",
    "\u{2713}",
    "\u{2705}",
];

/// Failure patterns indicating the agent encountered an error.
const FAILURE_PATTERNS: &[&str] = &[
    "Error:",
    "FATAL:",
    "panic:",
    "Permission denied",
    "command not found: claude",
];

/// Result of scanning a chunk of agent output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputSignal {
    /// Nothing notable detected.
    None,
    /// Token/rate limit hit — queue should pause.
    TokenLimitHit,
    /// The agent appears to have completed successfully.
    Completed,
    /// The agent encountered a fatal error.
    Failed(String),
}

/// Scan a chunk of terminal output for signals.
pub fn scan_output(text: &str) -> OutputSignal {
    // Check token limit patterns first (highest priority).
    for pattern in TOKEN_LIMIT_PATTERNS {
        if text.contains(pattern) {
            return OutputSignal::TokenLimitHit;
        }
    }

    // Check failure patterns.
    for pattern in FAILURE_PATTERNS {
        if text.contains(pattern) {
            return OutputSignal::Failed(
                text.lines()
                    .find(|line| line.contains(pattern))
                    .unwrap_or(pattern)
                    .to_string(),
            );
        }
    }

    OutputSignal::None
}

/// Build the command line to execute Claude Code with a task prompt.
///
/// Returns (program, args) suitable for spawning.
pub fn claude_command(prompt: &str, cwd: Option<&str>) -> (String, Vec<String>) {
    // Try to find claude in common locations.
    let claude_bin = which_claude().unwrap_or_else(|| "claude".to_string());

    let mut args = vec![
        "--print".to_string(),       // Non-interactive, print output
        "--dangerously-skip-permissions".to_string(), // Agent mode — no prompts
        "--output-format".to_string(),
        "json".to_string(),          // Structured output with usage data
    ];

    if let Some(dir) = cwd {
        args.push("--cwd".to_string());
        args.push(dir.to_string());
    }

    args.push(prompt.to_string());

    (claude_bin, args)
}

/// Build a shell command string that runs Claude Code with the given prompt.
///
/// This is suitable for feeding to a terminal via VTE spawn_async.
pub fn claude_shell_command(prompt: &str) -> String {
    let escaped = prompt.replace('\'', "'\\''");
    format!("claude --print --dangerously-skip-permissions '{escaped}'; echo '\\n[thane:plan:exit:$?]'")
}

/// Detect the exit marker in terminal output and extract the exit code.
///
/// Returns Some(exit_code) if the marker is found.
pub fn detect_exit_marker(text: &str) -> Option<i32> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("[thane:plan:exit:")
            && let Some(code_str) = rest.strip_suffix(']')
            && let Ok(code) = code_str.trim().parse::<i32>()
        {
            return Some(code);
        }
    }
    None
}

/// Try to find the `claude` binary.
pub fn which_claude() -> Option<String> {
    // Check common locations.
    #[cfg(target_os = "linux")]
    let candidates = [
        // npm global install
        "/usr/local/bin/claude",
        "/usr/bin/claude",
    ];
    #[cfg(target_os = "macos")]
    let candidates = [
        // npm global install
        "/usr/local/bin/claude",
        // Homebrew (Apple Silicon and Intel)
        "/opt/homebrew/bin/claude",
    ];

    for path in candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    // Check home-relative locations.
    if let Ok(home) = std::env::var("HOME") {
        let local_bin = format!("{home}/.local/bin/claude");
        if std::path::Path::new(&local_bin).exists() {
            return Some(local_bin);
        }
        let npm_global = format!("{home}/.npm-global/bin/claude");
        if std::path::Path::new(&npm_global).exists() {
            return Some(npm_global);
        }
        let nvm_default = format!("{home}/.nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(&nvm_default) {
            // Use the latest node version.
            let mut versions: Vec<_> = entries
                .filter_map(|e| e.ok())
                .collect();
            versions.sort_by_key(|b| std::cmp::Reverse(b.file_name()));
            if let Some(latest) = versions.first() {
                let claude_path = latest.path().join("bin/claude");
                if claude_path.exists() {
                    return Some(claude_path.to_string_lossy().to_string());
                }
            }
        }
    }

    None
}

/// Create a task directory `<base>/<uuid>/` and return its path.
///
/// Creates the directory tree if it doesn't exist.
pub fn task_dir(base: &str, entry_id: Uuid) -> PathBuf {
    let dir = PathBuf::from(base).join(entry_id.to_string());
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Shorten a Claude model name for display.
///
/// Common patterns:
/// - `"claude-3-5-sonnet-20241022"` -> `"3.5 Sonnet"`
/// - `"claude-sonnet-4-5-20250514"` -> `"Sonnet 4.5"`
/// - `"claude-opus-4-6"` -> `"Opus 4.6"`
/// - Unknown patterns -> returned as-is
pub fn shorten_model_name(model: &str) -> String {
    let Some(rest) = model.strip_prefix("claude-") else {
        return model.to_string();
    };

    let parts: Vec<&str> = rest.split('-').collect();

    // Pattern: claude-{major}-{minor}-{name}-{date}
    // e.g. "claude-3-5-sonnet-20241022" -> parts = ["3", "5", "sonnet", "20241022"]
    if parts.len() >= 3
        && let (Ok(major), Ok(minor)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>())
    {
        let name = capitalize(parts[2]);
        return format!("{major}.{minor} {name}");
    }

    // Pattern: claude-{name}-{major}-{minor}-{date} or claude-{name}-{major}-{minor}
    // e.g. "claude-sonnet-4-5-20250514" -> parts = ["sonnet", "4", "5", "20250514"]
    // e.g. "claude-opus-4-6" -> parts = ["opus", "4", "6"]
    if parts.len() >= 3
        && let (Ok(major), Ok(minor)) = (parts[1].parse::<u32>(), parts[2].parse::<u32>())
    {
        let name = capitalize(parts[0]);
        return format!("{name} {major}.{minor}");
    }

    model.to_string()
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Inject CLAUDE.md content into a prompt if the file exists in the given working directory.
///
/// If `<cwd>/CLAUDE.md` exists, its content is prepended to the prompt wrapped in
/// `<project-instructions>` tags. Otherwise the original prompt is returned unchanged.
pub fn inject_claude_md(prompt: &str, cwd: &str) -> String {
    let claude_md_path = std::path::Path::new(cwd).join("CLAUDE.md");
    match std::fs::read_to_string(&claude_md_path) {
        Ok(content) if !content.is_empty() => {
            format!(
                "<project-instructions>\n{content}\n</project-instructions>\n\n{prompt}"
            )
        }
        _ => prompt.to_string(),
    }
}

/// Spawn a headless Claude Code process for a queue task.
///
/// Runs `claude --print --dangerously-skip-permissions --cwd <cwd> <prompt>`
/// with stdout+stderr redirected to `log_path`.
///
/// Automatically injects CLAUDE.md content from the working directory into the prompt.
///
/// If `sandbox` is `Some` and enabled, sandbox environment variables are set
/// on the child process.
pub fn spawn_headless(
    prompt: &str,
    cwd: &str,
    log_path: &str,
    sandbox: Option<&SandboxPolicy>,
) -> Result<std::process::Child, std::io::Error> {
    let enriched_prompt = inject_claude_md(prompt, cwd);
    let (program, args) = claude_command(&enriched_prompt, None);

    let log_file = std::fs::File::create(log_path)?;
    let stderr_file = log_file.try_clone()?;

    let mut cmd = std::process::Command::new(&program);
    cmd.args(&args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(stderr_file))
        .stdin(std::process::Stdio::null());

    if let Some(policy) = sandbox {
        for (key, val) in policy.env_vars() {
            cmd.env(&key, &val);
        }
    }

    cmd.spawn()
}

/// Estimate when a rate limit will reset.
///
/// Parses common formats like "Please try again in 30 seconds" or
/// "retry after 60s". Falls back to 5 minutes if unparseable.
pub fn estimate_reset_time(output: &str) -> chrono::DateTime<Utc> {
    // Try to find "in N seconds/minutes" patterns.
    let lower = output.to_lowercase();

    if let Some(pos) = lower.find("try again in ") {
        let rest = &lower[pos + 13..];
        if let Some(seconds) = parse_duration_from_text(rest) {
            return Utc::now() + Duration::seconds(seconds);
        }
    }

    if let Some(pos) = lower.find("retry after ") {
        let rest = &lower[pos + 12..];
        if let Some(seconds) = parse_duration_from_text(rest) {
            return Utc::now() + Duration::seconds(seconds);
        }
    }

    // Default: assume 5-minute cooldown.
    Utc::now() + Duration::minutes(5)
}

/// Parse a duration from text like "30 seconds", "5 minutes", "60s", "2m".
fn parse_duration_from_text(text: &str) -> Option<i64> {
    let text = text.trim();

    // Try "Ns" or "Nm" format.
    if let Some(num_str) = text.strip_suffix('s')
        && let Ok(n) = num_str.trim().parse::<i64>()
    {
        return Some(n);
    }
    if let Some(num_str) = text.strip_suffix('m')
        && let Ok(n) = num_str.trim().parse::<i64>()
    {
        return Some(n * 60);
    }

    // Try "N seconds" or "N minutes" format.
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() >= 2
        && let Ok(n) = parts[0].parse::<i64>()
    {
        if parts[1].starts_with("second") {
            return Some(n);
        }
        if parts[1].starts_with("minute") {
            return Some(n * 60);
        }
        if parts[1].starts_with("hour") {
            return Some(n * 3600);
        }
    }

    None
}

/// Process output from a running queue entry and update the queue state accordingly.
///
/// Returns `true` if the entry is still running, `false` if it has finished.
pub fn process_queue_output(
    queue: &mut AgentQueue,
    entry_id: Uuid,
    output_chunk: &str,
) -> bool {
    // Check for exit marker first.
    if let Some(exit_code) = detect_exit_marker(output_chunk) {
        if exit_code == 0 {
            queue.complete(entry_id);
        } else {
            queue.fail(entry_id, format!("Process exited with code {exit_code}"));
        }
        return false;
    }

    // Scan for signals.
    match scan_output(output_chunk) {
        OutputSignal::TokenLimitHit => {
            let reset_time = estimate_reset_time(output_chunk);
            queue.pause_for_token_limit(reset_time);
            false
        }
        OutputSignal::Failed(msg) => {
            queue.fail(entry_id, msg);
            false
        }
        OutputSignal::Completed | OutputSignal::None => {
            // Still running.
            true
        }
    }
}

/// Parse token usage from a `claude --print --output-format json` output log.
///
/// The JSON output contains `total_cost_usd` and `usage.{input_tokens, output_tokens}`.
/// Returns `None` if the file can't be read or parsed.
pub fn parse_usage_from_log(log_path: &str) -> Option<QueueTokenUsage> {
    let content = std::fs::read_to_string(log_path).ok()?;
    parse_usage_from_json(&content)
}

/// Parse token usage from JSON content produced by `claude --print --output-format json`.
pub fn parse_usage_from_json(content: &str) -> Option<QueueTokenUsage> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;

    let input_tokens = v.get("usage")?.get("input_tokens")?.as_u64().unwrap_or(0);
    let output_tokens = v.get("usage")?.get("output_tokens")?.as_u64().unwrap_or(0);
    let cost = v.get("total_cost_usd")?.as_f64().unwrap_or(0.0);
    let model = v.get("model").and_then(|m| m.as_str()).map(|s| s.to_string());

    Some(QueueTokenUsage {
        input_tokens,
        output_tokens,
        estimated_cost_usd: cost,
        model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_queue::QueueEntryStatus;

    #[test]
    fn test_scan_output_token_limit() {
        assert_eq!(
            scan_output("Error: Rate limit reached. Please try again in 60 seconds."),
            OutputSignal::TokenLimitHit
        );
        assert_eq!(
            scan_output("429 Too Many Requests"),
            OutputSignal::TokenLimitHit
        );
    }

    #[test]
    fn test_scan_output_failure() {
        let signal = scan_output("Error: Permission denied accessing /etc/shadow");
        assert!(matches!(signal, OutputSignal::Failed(_)));
    }

    #[test]
    fn test_scan_output_normal() {
        assert_eq!(
            scan_output("Writing file src/main.rs..."),
            OutputSignal::None
        );
    }

    #[test]
    fn test_detect_exit_marker() {
        assert_eq!(detect_exit_marker("[thane:plan:exit:0]"), Some(0));
        assert_eq!(detect_exit_marker("[thane:plan:exit:1]"), Some(1));
        assert_eq!(
            detect_exit_marker("some output\n[thane:plan:exit:42]\nmore text"),
            Some(42)
        );
        assert_eq!(detect_exit_marker("no marker here"), None);
    }

    #[test]
    fn test_claude_shell_command() {
        let cmd = claude_shell_command("Fix the bug in main.rs");
        assert!(cmd.contains("claude --print"));
        assert!(cmd.contains("Fix the bug in main.rs"));
        assert!(cmd.contains("[thane:plan:exit:$?]"));
    }

    #[test]
    fn test_claude_shell_command_escapes_quotes() {
        let cmd = claude_shell_command("It's a test");
        // Single quotes should be escaped.
        assert!(cmd.contains("It"));
        assert!(cmd.contains("a test"));
    }

    #[test]
    fn test_estimate_reset_time_seconds() {
        let reset = estimate_reset_time("Please try again in 30 seconds.");
        let now = Utc::now();
        let diff = (reset - now).num_seconds();
        assert!(diff >= 28 && diff <= 32);
    }

    #[test]
    fn test_estimate_reset_time_minutes() {
        let reset = estimate_reset_time("retry after 2 minutes");
        let now = Utc::now();
        let diff = (reset - now).num_seconds();
        assert!(diff >= 118 && diff <= 122);
    }

    #[test]
    fn test_estimate_reset_time_default() {
        let reset = estimate_reset_time("some unparseable error");
        let now = Utc::now();
        let diff = (reset - now).num_seconds();
        // Default is 5 minutes = 300 seconds.
        assert!(diff >= 298 && diff <= 302);
    }

    #[test]
    fn test_parse_duration_from_text() {
        assert_eq!(parse_duration_from_text("30s"), Some(30));
        assert_eq!(parse_duration_from_text("2m"), Some(120));
        assert_eq!(parse_duration_from_text("30 seconds"), Some(30));
        assert_eq!(parse_duration_from_text("5 minutes"), Some(300));
        assert_eq!(parse_duration_from_text("1 hour"), Some(3600));
        assert_eq!(parse_duration_from_text("unparseable"), None);
    }

    #[test]
    fn test_process_queue_output_exit_success() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("test task".into(), None, 0);
        queue.start(id);

        let still_running = process_queue_output(&mut queue, id, "[thane:plan:exit:0]");
        assert!(!still_running);
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Completed);
    }

    #[test]
    fn test_process_queue_output_exit_failure() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("test task".into(), None, 0);
        queue.start(id);

        let still_running = process_queue_output(&mut queue, id, "[thane:plan:exit:1]");
        assert!(!still_running);
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Failed);
    }

    #[test]
    fn test_process_queue_output_token_limit() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("test task".into(), None, 0);
        queue.start(id);

        let still_running = process_queue_output(
            &mut queue,
            id,
            "Rate limit reached. Please try again in 60 seconds.",
        );
        assert!(!still_running);
        assert!(queue.token_limit_paused);
        assert_eq!(
            queue.get(id).unwrap().status,
            QueueEntryStatus::PausedTokenLimit
        );
    }

    #[test]
    fn test_process_queue_output_normal() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("test task".into(), None, 0);
        queue.start(id);

        let still_running = process_queue_output(&mut queue, id, "Writing files...\nOK");
        assert!(still_running);
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Running);
    }

    #[test]
    fn test_parse_usage_from_json() {
        let json = r#"{
            "type": "result",
            "subtype": "success",
            "model": "claude-sonnet-4-5-20250514",
            "total_cost_usd": 0.158,
            "usage": {
                "input_tokens": 25000,
                "output_tokens": 500
            }
        }"#;
        let usage = parse_usage_from_json(json).unwrap();
        assert_eq!(usage.input_tokens, 25000);
        assert_eq!(usage.output_tokens, 500);
        assert!((usage.estimated_cost_usd - 0.158).abs() < f64::EPSILON);
        assert_eq!(usage.model.as_deref(), Some("claude-sonnet-4-5-20250514"));
    }

    #[test]
    fn test_parse_usage_from_json_no_model() {
        let json = r#"{
            "type": "result",
            "total_cost_usd": 0.05,
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 200
            }
        }"#;
        let usage = parse_usage_from_json(json).unwrap();
        assert!(usage.model.is_none());
    }

    #[test]
    fn test_parse_usage_from_json_missing_fields() {
        assert!(parse_usage_from_json("not json").is_none());
        assert!(parse_usage_from_json(r#"{"type":"result"}"#).is_none());
    }

    #[test]
    fn test_shorten_model_name_old_style() {
        // claude-{major}-{minor}-{name}-{date}
        assert_eq!(shorten_model_name("claude-3-5-sonnet-20241022"), "3.5 Sonnet");
        assert_eq!(shorten_model_name("claude-3-5-haiku-20241022"), "3.5 Haiku");
        assert_eq!(shorten_model_name("claude-3-0-opus-20240229"), "3.0 Opus");
    }

    #[test]
    fn test_shorten_model_name_new_style() {
        // claude-{name}-{major}-{minor}-{date}
        assert_eq!(
            shorten_model_name("claude-sonnet-4-5-20250514"),
            "Sonnet 4.5"
        );
        assert_eq!(
            shorten_model_name("claude-haiku-4-5-20251001"),
            "Haiku 4.5"
        );
    }

    #[test]
    fn test_shorten_model_name_no_date() {
        // claude-{name}-{major}-{minor} (no date suffix)
        assert_eq!(shorten_model_name("claude-opus-4-6"), "Opus 4.6");
    }

    #[test]
    fn test_shorten_model_name_unknown() {
        // Unknown patterns returned as-is
        assert_eq!(shorten_model_name("gpt-4o"), "gpt-4o");
        assert_eq!(shorten_model_name("some-model"), "some-model");
        assert_eq!(shorten_model_name(""), "");
    }

    #[test]
    fn test_shorten_model_name_bare_claude() {
        // "claude-" prefix but no recognizable pattern
        assert_eq!(shorten_model_name("claude-unknown"), "claude-unknown");
    }

    #[test]
    fn test_inject_claude_md_when_present() {
        let dir = std::env::temp_dir().join(format!("thane-test-claude-md-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let claude_md = dir.join("CLAUDE.md");
        std::fs::write(&claude_md, "# Project\nBuild with cargo.").unwrap();

        let result = inject_claude_md("Fix the bug", dir.to_str().unwrap());
        assert!(result.starts_with("<project-instructions>"));
        assert!(result.contains("# Project"));
        assert!(result.contains("Build with cargo."));
        assert!(result.ends_with("Fix the bug"));

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_inject_claude_md_when_absent() {
        let dir = std::env::temp_dir().join(format!("thane-test-no-claude-md-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let result = inject_claude_md("Fix the bug", dir.to_str().unwrap());
        assert_eq!(result, "Fix the bug");

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_inject_claude_md_empty_file() {
        let dir = std::env::temp_dir().join(format!("thane-test-empty-claude-md-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let claude_md = dir.join("CLAUDE.md");
        std::fs::write(&claude_md, "").unwrap();

        let result = inject_claude_md("Fix the bug", dir.to_str().unwrap());
        assert_eq!(result, "Fix the bug");

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
