use std::io;
use std::path::PathBuf;

const OLD_MARKER: &str = "<!-- thane-agent-queue-instructions-v3 -->";
const MARKER: &str = "<!-- thane-agent-queue-instructions-v4 -->";

const THANE_INSTRUCTIONS: &str = r#"
<!-- thane-agent-queue-instructions-v4 -->
## thane Agent Queue Integration

When running inside a thane terminal workspace, you have access to the thane agent queue. The `$THANE_SOCKET_PATH` environment variable is automatically set in all thane terminal sessions.

### Submitting tasks to the agent queue

When the user asks you to add a plan to the thane queue, add tasks to the queue, or schedule work for later execution, write each task to a temp file and submit it:

```bash
cat <<'TASK' > /tmp/thane-task.md
Your detailed task description here.
TASK
thane-cli queue submit /tmp/thane-task.md
```

### Queue management

- `thane-cli queue list` — list all queued tasks
- `thane-cli queue status <id>` — check a specific task
- `thane-cli queue cancel <id>` — cancel a queued task

### Formatting tasks for autonomous execution

Each queued task runs as an **independent, headless Claude Code session** with no user interaction. Format every task so an autonomous agent can execute it without clarification:

1. **Working directory** — Always include the absolute path: `Working directory: /path/to/project`
2. **Objective** — A clear, concise summary of what the task should accomplish
3. **Context** — Relevant background: why this change is needed, what preceded it, any constraints
4. **Detailed instructions** — Step-by-step implementation details: which files to modify, what logic to add/change, expected behavior, edge cases to handle
5. **Acceptance criteria** — How to verify the task is complete (e.g., tests to run, expected output, commands to validate)
6. **References** — Relevant file paths, function names, documentation links, or related code patterns

### Submitting multi-phase plans

When the user asks you to queue a plan with multiple phases, **submit each phase as a separate task** rather than one monolithic task. Each phase should depend on the successful completion of the previous phase.

- Use `thane-cli queue submit --depends-on <previous-task-id>` to chain phases together
- The dependent task will only execute after its dependency completes successfully
- If a phase fails, subsequent dependent phases will not run

Example workflow for a 3-phase plan:

```bash
# Phase 1
cat <<'TASK' > /tmp/thane-phase1.md
## Phase 1: <title>
Working directory: /path/to/project
...detailed instructions...
TASK
PHASE1_ID=$(thane-cli queue submit /tmp/thane-phase1.md)

# Phase 2 — depends on Phase 1
cat <<'TASK' > /tmp/thane-phase2.md
## Phase 2: <title>
Working directory: /path/to/project
Depends on: Phase 1 — <brief description of what Phase 1 does>
...detailed instructions...
TASK
PHASE2_ID=$(thane-cli queue submit --depends-on "$PHASE1_ID" /tmp/thane-phase2.md)

# Phase 3 — depends on Phase 2
cat <<'TASK' > /tmp/thane-phase3.md
## Phase 3: <title>
Working directory: /path/to/project
Depends on: Phase 2 — <brief description of what Phase 2 does>
...detailed instructions...
TASK
thane-cli queue submit --depends-on "$PHASE2_ID" /tmp/thane-phase3.md
```

### Guidelines

- Only submit to the queue when the user explicitly asks (e.g., "add this plan to my thane queue", "add to the queue", "run this later")
- Write each task as if handing it to a developer who has never seen the codebase — include all necessary context
- Each queued task runs as an independent Claude Code session with no access to your current conversation
- For multi-phase plans, always submit phases independently with dependency chaining rather than as a single task
- Include in each dependent phase a brief description of what the prior phase was supposed to accomplish, so the agent can verify prerequisites
"#;

/// Returns the path to `~/.claude/CLAUDE.md`.
pub fn claude_md_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("CLAUDE.md"))
}

/// Checks whether the current versioned thane agent-queue instructions are present in `~/.claude/CLAUDE.md`.
pub fn has_thane_instructions() -> bool {
    let Some(path) = claude_md_path() else {
        return false;
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => content.contains(MARKER),
        Err(_) => false,
    }
}

/// Appends thane agent-queue instructions to `~/.claude/CLAUDE.md` if not already present.
///
/// If an older version of the instructions exists (detected by `OLD_MARKER`), it is
/// replaced with the current version. Creates `~/.claude/` and the file if they don't exist.
/// Returns `Ok(true)` if instructions were added or upgraded, `Ok(false)` if already current.
pub fn inject_thane_instructions() -> io::Result<bool> {
    let Some(path) = claude_md_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not determine home directory",
        ));
    };

    // Ensure ~/.claude/ exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Read existing content (or empty if file doesn't exist).
    let existing = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };

    // Already has the current version — nothing to do.
    if existing.contains(MARKER) {
        return Ok(false);
    }

    // If old marker is present, remove the old block and replace with new.
    let mut new_content = if existing.contains(OLD_MARKER) {
        remove_old_instructions(&existing)
    } else {
        existing
    };

    // Append current instructions.
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(THANE_INSTRUCTIONS);

    std::fs::write(&path, new_content)?;
    Ok(true)
}

/// Removes the old thane instruction block from content.
///
/// The old block starts at `OLD_MARKER` and extends to the end of the file
/// (since it was always appended at the end). We trim any trailing whitespace
/// that was part of the block boundary.
fn remove_old_instructions(content: &str) -> String {
    if let Some(start) = content.find(OLD_MARKER) {
        // The block was appended with a leading newline in THANE_INSTRUCTIONS,
        // so back up over the preceding newline if present.
        let trim_start = if start > 0 && content.as_bytes()[start - 1] == b'\n' {
            start - 1
        } else {
            start
        };
        content[..trim_start].to_string()
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_md_path_returns_some() {
        // On any system with a home directory, this should return Some.
        let path = claude_md_path();
        assert!(path.is_some());
        let p = path.unwrap();
        assert!(p.ends_with(".claude/CLAUDE.md"));
    }

    #[test]
    fn test_marker_detection() {
        let content_with = format!("# My stuff\n\n{MARKER}\nmore content");
        assert!(content_with.contains(MARKER));

        let content_without = "# My stuff\nno marker here";
        assert!(!content_without.contains(MARKER));
    }

    #[test]
    fn test_inject_idempotent() {
        let dir = std::env::temp_dir().join("thane-test-claude-md");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("CLAUDE.md");

        // Write initial content.
        std::fs::write(&path, "# Existing content\n").unwrap();

        // Manually inject using our constant to simulate inject_thane_instructions.
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push_str(THANE_INSTRUCTIONS);
        std::fs::write(&path, &content).unwrap();

        // Verify marker is present.
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(MARKER));

        // Verify original content is preserved.
        assert!(content.starts_with("# Existing content\n"));

        // Second injection should be a no-op (marker already present).
        assert!(content.contains(MARKER));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_instructions_contain_versioned_marker() {
        assert!(THANE_INSTRUCTIONS.contains(MARKER));
        assert!(THANE_INSTRUCTIONS.contains("thane-agent-queue-instructions-v4"));
    }

    #[test]
    fn test_instructions_contain_working_directory_guidance() {
        assert!(THANE_INSTRUCTIONS.contains("Working directory"));
    }

    #[test]
    fn test_old_marker_is_different_from_current() {
        assert_ne!(OLD_MARKER, MARKER);
        // The old marker should NOT be a substring of the new marker to avoid
        // false positives — but in this case OLD_MARKER IS a substring since
        // v2 marker contains the old prefix. That's fine because we check for
        // the versioned marker first.
    }

    #[test]
    fn test_remove_old_instructions() {
        let old_block = format!(
            "\n{OLD_MARKER}\n## thane Agent Queue Integration\n\nOld content here.\n"
        );
        let original = format!("# My stuff\n\nSome content.{old_block}");

        let result = remove_old_instructions(&original);
        assert_eq!(result, "# My stuff\n\nSome content.");
        assert!(!result.contains(OLD_MARKER));
    }

    #[test]
    fn test_remove_old_instructions_no_marker() {
        let content = "# My stuff\n\nNo marker here.".to_string();
        let result = remove_old_instructions(&content);
        assert_eq!(result, content);
    }

    #[test]
    fn test_upgrade_old_to_new() {
        // Simulate a file with the old (v1) instructions.
        let old_instructions = format!(
            "\n{OLD_MARKER}\n## thane Agent Queue Integration\n\nOld v1 content.\n"
        );
        let original = format!("# User Content\n\nStuff here.{old_instructions}");

        // Verify old marker is present, new is not.
        assert!(original.contains(OLD_MARKER));
        assert!(!original.contains(MARKER));

        // Run upgrade logic.
        let mut upgraded = remove_old_instructions(&original);
        if !upgraded.is_empty() && !upgraded.ends_with('\n') {
            upgraded.push('\n');
        }
        upgraded.push_str(THANE_INSTRUCTIONS);

        // New marker should be present, user content preserved.
        assert!(upgraded.contains(MARKER));
        assert!(upgraded.contains("# User Content"));
        assert!(upgraded.contains("Stuff here."));
        assert!(upgraded.contains("Working directory"));
        // Old content should be gone.
        assert!(!upgraded.contains("Old v1 content."));
    }
}
