//! Prompt capture — detect AI coding agent invocations in terminal I/O.
//!
//! Scans terminal input for patterns like:
//! - `claude "prompt text"` or `claude --print "prompt text"`
//! - `codex "prompt text"`
//! - `gemini "prompt text"`
//! - `aider "prompt text"`
//! - `goose "prompt text"`
//! - and other supported agents
//!
//! Captured prompts can be submitted to the agent queue for batch execution.

/// Known CLI agent binary names for prompt capture.
/// Order matters: longer prefixes checked first to avoid partial matches
/// (e.g., "claude-code" before "claude").
const AGENT_COMMANDS: &[&str] = &[
    "claude-code",
    "claude",
    "codex",
    "gemini",
    "goose",
    "opencode",
    "cline",
    "amp",
    "auggie",
    "openhands",
    "plandex",
    "qwen",
    "devin",
    "tabnine",
    "cursor",
    "aider",
    "copilot",
    "cody",
    "continue",
];

/// A captured agent prompt from terminal I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedPrompt {
    /// The name of the agent that was invoked (e.g., "claude", "codex", "aider").
    pub agent_name: String,
    /// The prompt text extracted from the command.
    pub text: String,
    /// Whether this was a --print / non-interactive invocation (agent-specific).
    pub print_mode: bool,
    /// The full command line that was detected.
    pub command_line: String,
}

/// Scan a line of terminal input for agent invocations.
///
/// Returns `Some(CapturedPrompt)` if a known agent command is detected with arguments.
/// ANSI escape sequences are stripped before matching.
pub fn detect_agent_prompt(line: &str) -> Option<CapturedPrompt> {
    let cleaned = crate::audit::strip_terminal_codes(line);
    let line = cleaned.trim();

    let (agent_name, args_start) = find_agent_command(line)?;
    let args_str = &line[args_start..];

    // Parse the arguments after the agent command.
    let parts = shell_split(args_str);
    if parts.is_empty() {
        return None;
    }

    let mut print_mode = false;
    let mut prompt_text = None;
    let mut skip_next = false;

    for part in &parts {
        if skip_next {
            skip_next = false;
            continue;
        }

        match part.as_str() {
            // Common print/non-interactive flags across agents
            "--print" | "-p" => print_mode = true,
            "--yes" | "-y" => {}                          // auto-confirm
            "--dangerously-skip-permissions" => {}         // claude-specific
            "--cwd" | "-c" => skip_next = true,           // working directory
            "--model" | "-m" => skip_next = true,          // model selection
            "--output-format" | "--format" => skip_next = true,
            "--config" => skip_next = true,
            "--provider" => skip_next = true,              // aider-specific
            "--api-key" => skip_next = true,               // various
            s if s.starts_with('-') => {}                  // skip other flags
            _ => {
                // First positional argument is the prompt text.
                if prompt_text.is_none() {
                    prompt_text = Some(part.clone());
                }
            }
        }
    }

    let text = prompt_text?;
    if text.is_empty() {
        return None;
    }

    Some(CapturedPrompt {
        agent_name: agent_name.to_string(),
        text,
        print_mode,
        command_line: line.to_string(),
    })
}

/// Backwards-compatible alias for `detect_agent_prompt`.
///
/// Existing callers that only care about Claude can continue to use this.
pub fn detect_claude_prompt(line: &str) -> Option<CapturedPrompt> {
    detect_agent_prompt(line)
}

/// Find an agent command in the line and return (agent_name, args_start_index).
///
/// Handles bare commands (`aider "prompt"`) and path-prefixed commands
/// (`/usr/local/bin/codex "prompt"`).
fn find_agent_command(line: &str) -> Option<(&'static str, usize)> {
    for &agent in AGENT_COMMANDS {
        // Check for `<agent> ` at the start of the line.
        let with_space = format!("{agent} ");
        if let Some(rest) = line.strip_prefix(&with_space) {
            return Some((agent, line.len() - rest.len()));
        }

        // Check for `/<agent> ` anywhere (path-prefixed).
        let path_pattern = format!("/{agent} ");
        if let Some(pos) = line.find(&path_pattern) {
            return Some((agent, pos + path_pattern.len()));
        }

        // Bare command with no args (interactive mode) — skip.
        if line == agent {
            return None;
        }
    }

    None
}

/// Simple shell argument splitting (handles single and double quotes).
fn shell_split(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in s.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Claude (existing tests, adapted) --

    #[test]
    fn test_detect_simple_prompt() {
        let result = detect_agent_prompt("claude \"Fix the bug in main.rs\"").unwrap();
        assert_eq!(result.agent_name, "claude");
        assert_eq!(result.text, "Fix the bug in main.rs");
        assert!(!result.print_mode);
    }

    #[test]
    fn test_detect_single_quoted() {
        let result = detect_agent_prompt("claude 'Add unit tests'").unwrap();
        assert_eq!(result.agent_name, "claude");
        assert_eq!(result.text, "Add unit tests");
    }

    #[test]
    fn test_detect_print_mode() {
        let result = detect_agent_prompt("claude --print 'Explain this code'").unwrap();
        assert_eq!(result.agent_name, "claude");
        assert_eq!(result.text, "Explain this code");
        assert!(result.print_mode);
    }

    #[test]
    fn test_detect_with_flags() {
        let result = detect_agent_prompt("claude --print --dangerously-skip-permissions 'Do the task'").unwrap();
        assert_eq!(result.text, "Do the task");
        assert!(result.print_mode);
    }

    #[test]
    fn test_detect_with_cwd() {
        let result = detect_agent_prompt("claude --cwd /home/user/project 'Build it'").unwrap();
        assert_eq!(result.text, "Build it");
    }

    #[test]
    fn test_detect_unquoted() {
        let result = detect_agent_prompt("claude hello");
        assert!(result.is_some());
        assert_eq!(result.unwrap().text, "hello");
    }

    #[test]
    fn test_detect_no_args() {
        // Interactive mode — no prompt to capture.
        assert!(detect_agent_prompt("claude").is_none());
    }

    #[test]
    fn test_detect_path_prefix() {
        let result = detect_agent_prompt("/usr/local/bin/claude 'Fix the tests'").unwrap();
        assert_eq!(result.agent_name, "claude");
        assert_eq!(result.text, "Fix the tests");
    }

    #[test]
    fn test_detect_non_claude() {
        assert!(detect_agent_prompt("git commit -m 'hello'").is_none());
    }

    #[test]
    fn test_detect_with_model_flag() {
        let result = detect_agent_prompt("claude --model opus 'Do something'").unwrap();
        assert_eq!(result.text, "Do something");
    }

    // -- Codex --

    #[test]
    fn test_detect_codex_prompt() {
        let result = detect_agent_prompt("codex \"Refactor the auth module\"").unwrap();
        assert_eq!(result.agent_name, "codex");
        assert_eq!(result.text, "Refactor the auth module");
        assert!(!result.print_mode);
    }

    #[test]
    fn test_detect_codex_no_args() {
        assert!(detect_agent_prompt("codex").is_none());
    }

    #[test]
    fn test_detect_codex_with_path() {
        let result = detect_agent_prompt("/opt/homebrew/bin/codex 'Fix tests'").unwrap();
        assert_eq!(result.agent_name, "codex");
        assert_eq!(result.text, "Fix tests");
    }

    // -- Gemini --

    #[test]
    fn test_detect_gemini_prompt() {
        let result = detect_agent_prompt("gemini 'Explain this function'").unwrap();
        assert_eq!(result.agent_name, "gemini");
        assert_eq!(result.text, "Explain this function");
    }

    // -- Aider --

    #[test]
    fn test_detect_aider_prompt() {
        let result = detect_agent_prompt("aider 'Add error handling to server.py'").unwrap();
        assert_eq!(result.agent_name, "aider");
        assert_eq!(result.text, "Add error handling to server.py");
    }

    #[test]
    fn test_detect_aider_with_provider() {
        let result = detect_agent_prompt("aider --provider openai 'Fix the bug'").unwrap();
        assert_eq!(result.agent_name, "aider");
        assert_eq!(result.text, "Fix the bug");
    }

    // -- Goose --

    #[test]
    fn test_detect_goose_prompt() {
        let result = detect_agent_prompt("goose 'Set up CI pipeline'").unwrap();
        assert_eq!(result.agent_name, "goose");
        assert_eq!(result.text, "Set up CI pipeline");
    }

    // -- Copilot --

    #[test]
    fn test_detect_copilot_prompt() {
        let result = detect_agent_prompt("copilot 'Generate unit tests for utils.ts'").unwrap();
        assert_eq!(result.agent_name, "copilot");
        assert_eq!(result.text, "Generate unit tests for utils.ts");
    }

    // -- Other agents --

    #[test]
    fn test_detect_opencode_prompt() {
        let result = detect_agent_prompt("opencode 'Refactor database layer'").unwrap();
        assert_eq!(result.agent_name, "opencode");
    }

    #[test]
    fn test_detect_cline_prompt() {
        let result = detect_agent_prompt("cline 'Add caching'").unwrap();
        assert_eq!(result.agent_name, "cline");
    }

    #[test]
    fn test_detect_amp_prompt() {
        let result = detect_agent_prompt("amp 'Search the codebase'").unwrap();
        assert_eq!(result.agent_name, "amp");
    }

    #[test]
    fn test_detect_devin_prompt() {
        let result = detect_agent_prompt("devin 'Fix failing tests'").unwrap();
        assert_eq!(result.agent_name, "devin");
    }

    #[test]
    fn test_detect_plandex_prompt() {
        let result = detect_agent_prompt("plandex 'Plan the migration'").unwrap();
        assert_eq!(result.agent_name, "plandex");
    }

    // -- claude-code vs claude disambiguation --

    #[test]
    fn test_detect_claude_code_before_claude() {
        // "claude-code" should match before "claude"
        let result = detect_agent_prompt("claude-code 'Do something'").unwrap();
        assert_eq!(result.agent_name, "claude-code");
    }

    // -- Backward compatibility --

    #[test]
    fn test_detect_claude_prompt_alias() {
        // The old function name still works
        let result = detect_claude_prompt("claude 'hello'").unwrap();
        assert_eq!(result.agent_name, "claude");
        assert_eq!(result.text, "hello");
    }

    // -- Non-agent commands --

    #[test]
    fn test_reject_random_commands() {
        assert!(detect_agent_prompt("npm install express").is_none());
        assert!(detect_agent_prompt("cargo build --release").is_none());
        assert!(detect_agent_prompt("ls -la").is_none());
        assert!(detect_agent_prompt("docker compose up").is_none());
    }

    // -- ANSI escape handling --

    #[test]
    fn test_detect_prompt_with_ansi_codes() {
        // Terminal may emit colored prompt text
        let result = detect_agent_prompt("\x1b[32mclaude\x1b[0m 'Fix the bug'").unwrap();
        assert_eq!(result.agent_name, "claude");
        assert_eq!(result.text, "Fix the bug");
    }

    #[test]
    fn test_detect_prompt_with_bold_ansi() {
        let result = detect_agent_prompt("\x1b[1mcodex\x1b[0m \"Refactor auth\"").unwrap();
        assert_eq!(result.agent_name, "codex");
        assert_eq!(result.text, "Refactor auth");
    }

    #[test]
    fn test_detect_prompt_with_osc_title_prefix() {
        // OSC title setting followed by actual command
        let result = detect_agent_prompt("\x1b]0;~/project\x07claude 'Do stuff'").unwrap();
        assert_eq!(result.agent_name, "claude");
        assert_eq!(result.text, "Do stuff");
    }

    // -- Edge cases --

    #[test]
    fn test_detect_prompt_empty_quoted_string() {
        // Empty string prompt should return None
        assert!(detect_agent_prompt("claude ''").is_none());
        assert!(detect_agent_prompt("claude \"\"").is_none());
    }

    #[test]
    fn test_detect_prompt_only_flags_no_prompt() {
        // Only flags, no positional prompt argument
        assert!(detect_agent_prompt("claude --print --model opus").is_none());
    }

    #[test]
    fn test_detect_prompt_whitespace_only() {
        assert!(detect_agent_prompt("   ").is_none());
        assert!(detect_agent_prompt("").is_none());
    }

    #[test]
    fn test_detect_prompt_captures_command_line() {
        let result = detect_agent_prompt("claude --print 'Hello world'").unwrap();
        assert_eq!(result.command_line, "claude --print 'Hello world'");
    }

    #[test]
    fn test_detect_all_supported_agents() {
        // Verify every agent in AGENT_COMMANDS can be detected
        for &agent in AGENT_COMMANDS {
            let line = format!("{agent} 'test prompt'");
            let result = detect_agent_prompt(&line);
            assert!(result.is_some(), "Failed to detect agent: {agent}");
            assert_eq!(result.unwrap().agent_name, agent,
                "Wrong agent name for: {agent}");
        }
    }

    // -- Shell splitting --

    #[test]
    fn test_shell_split() {
        assert_eq!(
            shell_split("--print 'hello world' --flag"),
            vec!["--print", "hello world", "--flag"]
        );
        assert_eq!(
            shell_split("\"hello world\" foo"),
            vec!["hello world", "foo"]
        );
        assert_eq!(
            shell_split("hello\\ world"),
            vec!["hello world"]
        );
    }
}
