use crate::sidebar::AgentStatus;

/// Known agent process names to look for in child process trees.
///
/// Each entry is `(name, exact)`:
/// - `exact = false` → substring match (safe for longer names)
/// - `exact = true`  → the process name must equal this exactly (for short
///   names like "amp" that would otherwise match "trampoline", "sample", etc.)
const AGENT_PROCESS_NAMES: &[(&str, bool)] = &[
    ("claude-code", false),
    ("claude", false),
    ("codex", false),
    ("gemini", false),
    ("goose", false),
    ("opencode", false),
    ("cline", false),
    ("amp", true),       // exact — too short for substring
    ("auggie", false),
    ("openhands", false),
    ("plandex", false),
    ("qwen", false),
    ("devin", false),
    ("tabnine", false),
    ("cursor", false),
    ("aider", false),
    ("copilot", false),
    ("cody", true),      // exact — avoid matching "codyze", etc.
    ("continue", false),
];

/// Result of agent detection for a PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDetection {
    pub status: AgentStatus,
    /// The name of the detected agent (e.g. "claude", "codex"), if any.
    pub agent_name: Option<String>,
}

/// Detect if an agent process is running as a descendant of the given PID.
/// Returns the agent status and the name of the detected agent.
pub fn detect_agent_for_pid(shell_pid: Option<i32>) -> AgentDetection {
    let pid = match shell_pid {
        Some(p) if p > 0 => p,
        _ => return AgentDetection { status: AgentStatus::Inactive, agent_name: None },
    };

    // Walk the process tree under this PID looking for agent processes.
    let children = collect_descendant_pids(pid);
    let names: Vec<String> = children
        .into_iter()
        .filter_map(read_process_name)
        .collect();
    detect_agent_from_process_names(&names)
}

/// Detect an agent given a list of process names (e.g. from a process tree).
/// This is the testable core of agent detection — separated from OS-specific
/// process tree walking so it can be exercised with mock process lists.
pub fn detect_agent_from_process_names(names: &[String]) -> AgentDetection {
    for name in names {
        if let Some(agent_name) = match_agent_process(name) {
            return AgentDetection {
                status: AgentStatus::Active,
                agent_name: Some(agent_name.to_string()),
            };
        }
    }
    AgentDetection { status: AgentStatus::Inactive, agent_name: None }
}

/// Check whether a process name matches any known agent.
/// Returns the canonical agent name if matched.
fn match_agent_process(name: &str) -> Option<&'static str> {
    let name_lower = name.to_lowercase();
    for &(agent_name, exact) in AGENT_PROCESS_NAMES {
        if exact {
            if name_lower == agent_name {
                return Some(agent_name);
            }
        } else if name_lower.contains(agent_name) {
            return Some(agent_name);
        }
    }
    None
}

/// Collect all descendant PIDs of a given PID by walking /proc.
#[cfg(target_os = "linux")]
pub fn collect_descendant_pids(parent_pid: i32) -> Vec<i32> {
    let mut result = Vec::new();
    let mut queue = vec![parent_pid];

    while let Some(pid) = queue.pop() {
        // Read /proc/<pid>/task/<tid>/children or scan /proc for ppid matches.
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        if let Ok(content) = std::fs::read_to_string(&children_path) {
            for token in content.split_whitespace() {
                if let Ok(child_pid) = token.parse::<i32>() {
                    result.push(child_pid);
                    queue.push(child_pid);
                }
            }
        }
    }

    result
}

/// Collect all descendant PIDs of a given PID using libproc.
#[cfg(target_os = "macos")]
pub fn collect_descendant_pids(parent_pid: i32) -> Vec<i32> {
    use libproc::processes::{pids_by_type, ProcFilter};

    let mut result = Vec::new();
    let mut queue = vec![parent_pid];

    while let Some(pid) = queue.pop() {
        if let Ok(children) = pids_by_type(ProcFilter::ByParentProcess { ppid: pid as u32 }) {
            for child_pid in children {
                let child = child_pid as i32;
                result.push(child);
                queue.push(child);
            }
        }
    }

    result
}

/// Read the process name (comm) for a PID from /proc.
#[cfg(target_os = "linux")]
fn read_process_name(pid: i32) -> Option<String> {
    let comm_path = format!("/proc/{pid}/comm");
    std::fs::read_to_string(comm_path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Read the process name for a PID using libproc.
#[cfg(target_os = "macos")]
fn read_process_name(pid: i32) -> Option<String> {
    libproc::libproc::proc_pid::name(pid).ok()
}

/// Read the command line for a PID from /proc.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
fn read_process_cmdline(pid: i32) -> Option<String> {
    let path = format!("/proc/{pid}/cmdline");
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.replace('\0', " ").trim().to_string())
}

/// Read the executable path for a PID using libproc.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn read_process_cmdline(pid: i32) -> Option<String> {
    libproc::libproc::proc_pid::pidpath(pid).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper — convenience wrapper around `match_agent_process`.
    fn is_agent_process(name: &str) -> bool {
        match_agent_process(name).is_some()
    }

    #[test]
    fn test_no_pid() {
        assert_eq!(detect_agent_for_pid(None).status, AgentStatus::Inactive);
        assert_eq!(detect_agent_for_pid(Some(0)).status, AgentStatus::Inactive);
        assert_eq!(detect_agent_for_pid(Some(-1)).status, AgentStatus::Inactive);
        assert!(detect_agent_for_pid(None).agent_name.is_none());
    }

    #[test]
    fn test_nonexistent_pid() {
        let detection = detect_agent_for_pid(Some(999_999_999));
        assert_eq!(detection.status, AgentStatus::Inactive);
        assert!(detection.agent_name.is_none());
    }

    #[test]
    fn test_collect_descendants_nonexistent() {
        let children = collect_descendant_pids(999_999_999);
        assert!(children.is_empty());
    }

    #[test]
    fn test_is_agent_process_substring_match() {
        // Longer names use substring matching
        assert!(is_agent_process("claude"));
        assert!(is_agent_process("claude-code"));
        assert!(is_agent_process("codex"));
        assert!(is_agent_process("gemini"));
        assert!(is_agent_process("goose"));
        assert!(is_agent_process("opencode"));
        assert!(is_agent_process("cline"));
        assert!(is_agent_process("auggie"));
        assert!(is_agent_process("openhands"));
        assert!(is_agent_process("plandex"));
        assert!(is_agent_process("qwen"));
        assert!(is_agent_process("devin"));
        assert!(is_agent_process("tabnine"));
        assert!(is_agent_process("cursor"));
        assert!(is_agent_process("aider"));
        assert!(is_agent_process("copilot"));
        assert!(is_agent_process("continue"));
    }

    #[test]
    fn test_is_agent_process_case_insensitive() {
        assert!(is_agent_process("Claude"));
        assert!(is_agent_process("CODEX"));
        assert!(is_agent_process("Gemini"));
        assert!(is_agent_process("AMP"));
    }

    #[test]
    fn test_is_agent_process_exact_match_short_names() {
        // "amp" requires exact match
        assert!(is_agent_process("amp"));
        assert!(!is_agent_process("trampoline"));
        assert!(!is_agent_process("sample"));
        assert!(!is_agent_process("amplify"));
        assert!(!is_agent_process("lamp"));

        // "cody" requires exact match
        assert!(is_agent_process("cody"));
        assert!(!is_agent_process("codyze"));
        assert!(!is_agent_process("codytools"));
    }

    #[test]
    fn test_is_agent_process_rejects_unknown() {
        assert!(!is_agent_process("vim"));
        assert!(!is_agent_process("bash"));
        assert!(!is_agent_process("node"));
        assert!(!is_agent_process("python3"));
        assert!(!is_agent_process("git"));
    }

    #[test]
    fn test_match_agent_process_returns_canonical_name() {
        // Substring agents return the canonical lowercase name
        assert_eq!(match_agent_process("Claude"), Some("claude"));
        assert_eq!(match_agent_process("CODEX"), Some("codex"));
        assert_eq!(match_agent_process("Gemini-Pro"), Some("gemini"));
        assert_eq!(match_agent_process("my-aider-wrapper"), Some("aider"));

        // Exact-match agents return their name too
        assert_eq!(match_agent_process("amp"), Some("amp"));
        assert_eq!(match_agent_process("AMP"), Some("amp"));
        assert_eq!(match_agent_process("cody"), Some("cody"));
        assert_eq!(match_agent_process("CODY"), Some("cody"));

        // Unknown returns None
        assert_eq!(match_agent_process("vim"), None);
        assert_eq!(match_agent_process(""), None);
    }

    #[test]
    fn test_match_agent_process_claude_code_before_claude() {
        // "claude-code" should match as "claude-code", not just "claude"
        assert_eq!(match_agent_process("claude-code"), Some("claude-code"));
        // Plain "claude" still matches as "claude"
        assert_eq!(match_agent_process("claude"), Some("claude"));
    }

    #[test]
    fn test_match_agent_process_embedded_substring() {
        // Substring agents match when embedded in longer names
        assert!(is_agent_process("node-claude-runner"));
        assert!(is_agent_process("copilot-cli"));
        assert!(is_agent_process("my-cursor-fork"));

        // But exact-match agents don't match when embedded
        assert!(!is_agent_process("preamp"));
        assert!(!is_agent_process("vamp"));
        assert!(!is_agent_process("encode"));  // doesn't contain "cody"
    }

    #[test]
    fn test_detect_agent_for_pid_returns_agent_name_none_for_invalid() {
        // Ensure agent_name is None for all invalid PID variants
        assert_eq!(detect_agent_for_pid(None).agent_name, None);
        assert_eq!(detect_agent_for_pid(Some(0)).agent_name, None);
        assert_eq!(detect_agent_for_pid(Some(-42)).agent_name, None);
        assert_eq!(detect_agent_for_pid(Some(999_999_999)).agent_name, None);
    }

    // ── detect_agent_from_process_names tests ────────────────────────────

    fn names(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_detect_from_names_empty_list() {
        let det = detect_agent_from_process_names(&[]);
        assert_eq!(det.status, AgentStatus::Inactive);
        assert!(det.agent_name.is_none());
    }

    #[test]
    fn test_detect_from_names_no_agents() {
        let det = detect_agent_from_process_names(&names(&[
            "bash", "node", "python3", "git", "npm",
        ]));
        assert_eq!(det.status, AgentStatus::Inactive);
        assert!(det.agent_name.is_none());
    }

    #[test]
    fn test_detect_each_known_agent() {
        // Every supported agent should be detected when present alongside
        // non-agent processes, and should return the correct canonical name.
        let cases: &[(&str, &str)] = &[
            ("claude-code", "claude-code"),
            ("claude", "claude"),
            ("codex", "codex"),
            ("gemini", "gemini"),
            ("goose", "goose"),
            ("opencode", "opencode"),
            ("cline", "cline"),
            ("amp", "amp"),
            ("auggie", "auggie"),
            ("openhands", "openhands"),
            ("plandex", "plandex"),
            ("qwen", "qwen"),
            ("devin", "devin"),
            ("tabnine", "tabnine"),
            ("cursor", "cursor"),
            ("aider", "aider"),
            ("copilot", "copilot"),
            ("cody", "cody"),
            ("continue", "continue"),
        ];

        for &(process_name, expected_canonical) in cases {
            let det = detect_agent_from_process_names(&names(&[
                "bash", process_name, "node",
            ]));
            assert_eq!(
                det.status,
                AgentStatus::Active,
                "Expected Active for process '{process_name}'"
            );
            assert_eq!(
                det.agent_name.as_deref(),
                Some(expected_canonical),
                "Wrong canonical name for process '{process_name}'"
            );
        }
    }

    #[test]
    fn test_detect_returns_first_agent_found() {
        // When multiple agents are in the tree, the first one wins.
        let det = detect_agent_from_process_names(&names(&[
            "bash", "claude", "codex", "gemini",
        ]));
        assert_eq!(det.status, AgentStatus::Active);
        assert_eq!(det.agent_name.as_deref(), Some("claude"));
    }

    #[test]
    fn test_detect_claude_code_preferred_over_claude() {
        // "claude-code" should match as "claude-code", not fall through to "claude"
        let det = detect_agent_from_process_names(&names(&[
            "bash", "claude-code",
        ]));
        assert_eq!(det.agent_name.as_deref(), Some("claude-code"));
    }

    #[test]
    fn test_detect_case_insensitive_in_tree() {
        let det = detect_agent_from_process_names(&names(&[
            "zsh", "GEMINI",
        ]));
        assert_eq!(det.status, AgentStatus::Active);
        assert_eq!(det.agent_name.as_deref(), Some("gemini"));
    }

    #[test]
    fn test_detect_exact_match_agents_reject_substrings() {
        // "amp" embedded in other process names should not trigger detection.
        let det = detect_agent_from_process_names(&names(&[
            "bash", "trampoline", "sample", "lamp",
        ]));
        assert_eq!(det.status, AgentStatus::Inactive);

        // "cody" embedded in other names likewise.
        let det = detect_agent_from_process_names(&names(&[
            "bash", "codyze", "codytools",
        ]));
        assert_eq!(det.status, AgentStatus::Inactive);
    }

    #[test]
    fn test_detect_substring_agents_match_wrappers() {
        // Substring agents should be found even inside wrapper process names.
        let det = detect_agent_from_process_names(&names(&[
            "zsh", "node-claude-runner",
        ]));
        assert_eq!(det.agent_name.as_deref(), Some("claude"));

        let det = detect_agent_from_process_names(&names(&[
            "bash", "copilot-cli",
        ]));
        assert_eq!(det.agent_name.as_deref(), Some("copilot"));

        let det = detect_agent_from_process_names(&names(&[
            "bash", "my-cursor-fork",
        ]));
        assert_eq!(det.agent_name.as_deref(), Some("cursor"));
    }

    #[test]
    fn test_detect_agent_deeply_nested_in_tree() {
        // Agent is far down the list (simulating deep process tree).
        let det = detect_agent_from_process_names(&names(&[
            "bash", "zsh", "tmux", "node", "npm", "sh", "python3", "aider",
        ]));
        assert_eq!(det.status, AgentStatus::Active);
        assert_eq!(det.agent_name.as_deref(), Some("aider"));
    }
}
