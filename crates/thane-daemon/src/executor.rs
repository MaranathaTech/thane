//! Queue executor loop: polls for runnable queue entries and spawns
//! Claude Code as a child process to handle each one.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use thane_core::queue_executor;
use thane_platform::traits::PlatformDirs;
use uuid::Uuid;

use crate::DaemonState;
use crate::platform_dirs;

/// Sleep between queue polls when nothing is runnable. Matches the cadence
/// of the GUI's `queuePollTimer` (2s).
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Run the executor loop until cancelled.
pub async fn run_loop(state: Arc<DaemonState>) {
    loop {
        // Look for a runnable entry under the state lock.
        let next = state.with_queue(|q| {
            q.check_token_limit_reset();
            q.next_runnable().cloned()
        });

        let Some(entry) = next else {
            tokio::time::sleep(IDLE_POLL_INTERVAL).await;
            continue;
        };

        // Mark running.
        state.with_queue(|q| {
            q.start(entry.id);
        });

        let prompt = entry.content.clone();
        let entry_id = entry.id;

        match run_task(entry_id, &prompt).await {
            TaskOutcome::Completed(usage) => {
                state.with_queue(|q| {
                    if let Some(u) = usage {
                        q.update_tokens(entry_id, u);
                    }
                    q.complete(entry_id);
                });
                tracing::info!("task {entry_id} completed");
            }
            TaskOutcome::Failed(msg) => {
                state.with_queue(|q| {
                    q.fail(entry_id, msg.clone());
                });
                tracing::warn!("task {entry_id} failed: {msg}");
            }
            TaskOutcome::TokenLimitHit(combined) => {
                let reset = queue_executor::estimate_reset_time(&combined);
                state.with_queue(|q| {
                    q.pause_for_token_limit(reset);
                });
                tracing::warn!("token limit hit, pausing until {reset}");
            }
        }
    }
}

enum TaskOutcome {
    Completed(Option<thane_core::agent_queue::QueueTokenUsage>),
    Failed(String),
    TokenLimitHit(String),
}

/// Spawn Claude Code for a single queue entry and wait for it to finish.
async fn run_task(entry_id: Uuid, prompt: &str) -> TaskOutcome {
    let task_dir = task_directory(entry_id);
    if let Err(e) = std::fs::create_dir_all(&task_dir) {
        return TaskOutcome::Failed(format!("create task dir: {e}"));
    }

    // Mirror the Swift GUI by writing the prompt to prompt.md in the task dir.
    let prompt_file = task_dir.join("prompt.md");
    let _ = std::fs::write(&prompt_file, prompt);

    let cwd_str = task_dir.to_string_lossy().into_owned();
    let (program, args) = queue_executor::claude_command(prompt, Some(&cwd_str));

    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args)
        .current_dir(&cwd_str)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("THANE_QUEUE_ENTRY_ID", entry_id.to_string());

    // Augment PATH so common claude install locations are found by the child.
    if let Some(path) = augmented_path() {
        cmd.env("PATH", path);
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return TaskOutcome::Failed(format!("spawn claude: {e}")),
    };

    let output = match child.wait_with_output().await {
        Ok(o) => o,
        Err(e) => return TaskOutcome::Failed(format!("wait claude: {e}")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    // Persist the raw JSON output so the GUI's log readers can still pick it up.
    let plans_log = plans_log_path(entry_id);
    if let Some(parent) = plans_log.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let combined = format!("{stdout}{stderr}");
    let _ = std::fs::write(&plans_log, &stdout);

    // Token limit check has priority over success/failure.
    if let queue_executor::OutputSignal::TokenLimitHit = queue_executor::scan_output(&combined) {
        return TaskOutcome::TokenLimitHit(combined);
    }

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        return TaskOutcome::Failed(format!("exit code {code}"));
    }

    let usage = queue_executor::parse_usage_from_json(&stdout);
    TaskOutcome::Completed(usage)
}

/// `~/thane-tasks/<entry-id>/` — matches the GUI's layout.
fn task_directory(entry_id: Uuid) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("thane-tasks").join(entry_id.to_string())
}

/// `<sessions_dir>/plans/<entry-id>/output.log`.
fn plans_log_path(entry_id: Uuid) -> PathBuf {
    let dirs = platform_dirs();
    dirs.data_dir()
        .join("plans")
        .join(entry_id.to_string())
        .join("output.log")
}

/// Mirror of Swift `augmentedPath`: prepend common install locations so the
/// child process can find `claude`, `node`, etc. even when launched by
/// launchd / systemd with a minimal PATH.
fn augmented_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let existing = std::env::var("PATH").unwrap_or_default();
    let prepend = [
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
        format!("{home}/.local/bin"),
        format!("{home}/.cargo/bin"),
    ];
    let mut parts: Vec<String> = prepend.to_vec();
    if !existing.is_empty() {
        for p in existing.split(':') {
            if !parts.contains(&p.to_string()) {
                parts.push(p.to_string());
            }
        }
    }
    Some(parts.join(":"))
}
