use clap::Parser;
use thane_platform::traits::PlatformDirs;
use tracing_subscriber::EnvFilter;

/// thane — AI-native terminal workspace manager.
#[derive(Parser)]
#[command(name = "thane", version, about)]
struct Cli {
    /// Run in headless mode (no GUI) — execute tasks from the queue.
    #[arg(long)]
    headless: bool,

    /// Working directory for headless task execution.
    #[arg(long, default_value = ".")]
    cwd: String,
}

/// Get the platform-specific directory provider.
#[cfg(target_os = "linux")]
fn platform_dirs() -> thane_platform::LinuxDirs {
    thane_platform::LinuxDirs
}

#[cfg(target_os = "macos")]
fn platform_dirs() -> thane_platform::MacosDirs {
    thane_platform::MacosDirs
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let dirs = platform_dirs();

    // Ensure all required directories exist before anything else.
    if let Err(e) = dirs.ensure_dirs() {
        tracing::error!("Failed to create directories: {e}");
    }

    let cli = Cli::parse();

    if cli.headless {
        tracing::info!("Starting thane in headless mode");
        run_headless(&cli.cwd);
    } else {
        // Acquire single-instance lock before starting the GUI.
        let runtime_dir = dirs.runtime_dir();
        let _lock = match thane_platform::pidlock::PidLock::acquire(&runtime_dir) {
            Ok(lock) => lock,
            Err(e) => {
                eprintln!("thane: {e}");
                std::process::exit(1);
            }
        };

        tracing::info!("Starting thane");
        thane_gtk::run();
    }
}

/// Run thane in headless mode — no GUI, just task execution via socket API.
fn run_headless(cwd: &str) {
    use std::sync::Arc;

    use thane_core::queue_executor;
    use thane_core::agent_queue::AgentQueue;
    use thane_ipc::auth::AccessMode;
    use thane_platform::traits::PlatformDirs;

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let queue = Arc::new(std::sync::Mutex::new(AgentQueue::new()));
    let cwd = cwd.to_string();

    rt.block_on(async {
        // Start the IPC socket server for task submissions.
        let dirs = platform_dirs();
        let socket_path = dirs.socket_path();

        // Clean up stale socket.
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        let queue_ref = queue.clone();
        let handler: thane_ipc::client::AsyncRpcHandler = Arc::new(move |request: thane_rpc::protocol::RpcRequest| {
            let queue_ref = queue_ref.clone();
            Box::pin(async move {
                handle_headless_rpc(&queue_ref, &request)
            })
        });

        // Spawn the socket server.
        let server_handle = tokio::spawn({
            let socket_path = socket_path.clone();
            async move {
                if let Err(e) = thane_ipc::server::start_server(
                    &socket_path,
                    handler,
                    AccessMode::Open,
                )
                .await
                {
                    tracing::error!("Socket server error: {e}");
                }
            }
        });

        tracing::info!("Headless mode: socket at {}", socket_path.display());
        tracing::info!("Submit tasks via: thane-cli queue submit <file>");

        // Main execution loop: poll the queue and run tasks.
        let queue_ref = queue.clone();
        let executor_handle = tokio::spawn(async move {
            loop {
                // Check for runnable entries.
                let entry = {
                    let mut q = queue_ref.lock().unwrap();
                    q.check_token_limit_reset();
                    q.next_runnable().cloned()
                };

                if let Some(entry) = entry {
                    // Mark as running.
                    {
                        let mut q = queue_ref.lock().unwrap();
                        q.start(entry.id);
                    }

                    tracing::info!("Executing task {}: {}", entry.id, &entry.content[..entry.content.len().min(50)]);

                    // Spawn Claude Code as a child process.
                    let (program, args) = queue_executor::claude_command(&entry.content, Some(&cwd));

                    match tokio::process::Command::new(&program)
                        .args(&args)
                        .current_dir(&cwd)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                    {
                        Ok(child) => {
                            // Read output and monitor for signals.
                            let output = child.wait_with_output().await;
                            match output {
                                Ok(output) => {
                                    let stdout = String::from_utf8_lossy(&output.stdout);
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    let combined = format!("{stdout}\n{stderr}");

                                    // Print output.
                                    if !stdout.is_empty() {
                                        print!("{stdout}");
                                    }
                                    if !stderr.is_empty() {
                                        eprint!("{stderr}");
                                    }

                                    // Check for token limit in output.
                                    let signal = queue_executor::scan_output(&combined);
                                    let mut q = queue_ref.lock().unwrap();

                                    match signal {
                                        queue_executor::OutputSignal::TokenLimitHit => {
                                            let reset = queue_executor::estimate_reset_time(&combined);
                                            tracing::warn!("Token limit hit, pausing until {reset}");
                                            q.pause_for_token_limit(reset);
                                        }
                                        _ => {
                                            if output.status.success() {
                                                q.complete(entry.id);
                                                tracing::info!("Task {} completed successfully", entry.id);
                                            } else {
                                                let code = output.status.code().unwrap_or(-1);
                                                q.fail(entry.id, format!("Exit code {code}"));
                                                tracing::error!("Task {} failed with exit code {code}", entry.id);
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    let mut q = queue_ref.lock().unwrap();
                                    q.fail(entry.id, format!("Process error: {e}"));
                                    tracing::error!("Task {} process error: {e}", entry.id);
                                }
                            }
                        }
                        Err(e) => {
                            let mut q = queue_ref.lock().unwrap();
                            q.fail(entry.id, format!("Failed to spawn claude: {e}"));
                            tracing::error!("Failed to spawn claude for task {}: {e}", entry.id);
                        }
                    }
                } else {
                    // No runnable entries — sleep and retry.
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        });

        // Wait for either task to finish (they shouldn't unless error).
        tokio::select! {
            _ = server_handle => tracing::error!("Socket server exited unexpectedly"),
            _ = executor_handle => tracing::error!("Queue executor exited unexpectedly"),
        }
    });
}

/// Handle RPC requests in headless mode (only agent queue methods).
fn handle_headless_rpc(
    queue: &std::sync::Mutex<thane_core::agent_queue::AgentQueue>,
    request: &thane_rpc::protocol::RpcRequest,
) -> thane_rpc::protocol::RpcResponse {
    use thane_rpc::methods::Method;
    use thane_rpc::protocol::RpcResponse;
    use serde_json::{json, Value};

    let id = request.id.clone();

    let method = match Method::parse(&request.method) {
        Some(m) => m,
        None => {
            return RpcResponse::error(
                id,
                -32601,
                format!("Method not found: {} (headless mode only supports agent_queue.*)", request.method),
            );
        }
    };

    let params = &request.params;

    match method {
        Method::AgentQueueSubmit => {
            let content = params["content"].as_str().unwrap_or("").to_string();
            if content.is_empty() {
                return RpcResponse::invalid_params(id, "Missing 'content' parameter");
            }
            let workspace_id = params["workspace_id"]
                .as_str()
                .and_then(|s| s.parse::<uuid::Uuid>().ok());
            let priority = params["priority"].as_i64().unwrap_or(0) as i32;

            let mut q = queue.lock().unwrap();
            let entry_id = q.submit(content, workspace_id, priority);
            RpcResponse::success(id, json!({ "entry_id": entry_id }))
        }

        Method::AgentQueueList => {
            let q = queue.lock().unwrap();
            let entries: Vec<Value> = q
                .list()
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "status": p.status,
                        "priority": p.priority,
                        "created_at": p.created_at.to_rfc3339(),
                        "content_preview": &p.content[..p.content.len().min(100)],
                    })
                })
                .collect();
            RpcResponse::success(
                id,
                json!({
                    "entries": entries,
                    "token_limit_paused": q.token_limit_paused,
                    "queued_count": q.queued_count(),
                    "running_count": q.running_count(),
                }),
            )
        }

        Method::AgentQueueStatus => {
            let entry_id = match params["entry_id"].as_str().and_then(|s| s.parse().ok()) {
                Some(id) => id,
                None => return RpcResponse::invalid_params(id, "Missing or invalid entry_id"),
            };
            let q = queue.lock().unwrap();
            match q.get(entry_id) {
                Some(p) => RpcResponse::success(
                    id,
                    json!({
                        "id": p.id,
                        "status": p.status,
                        "content": p.content,
                        "priority": p.priority,
                        "created_at": p.created_at.to_rfc3339(),
                        "started_at": p.started_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                        "completed_at": p.completed_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                        "error": p.error,
                        "tokens_used": {
                            "input_tokens": p.tokens_used.input_tokens,
                            "output_tokens": p.tokens_used.output_tokens,
                            "estimated_cost_usd": p.tokens_used.estimated_cost_usd,
                        },
                    }),
                ),
                None => RpcResponse::error(id, -1, "Entry not found"),
            }
        }

        Method::AgentQueueCancel => {
            let entry_id = match params["entry_id"].as_str().and_then(|s| s.parse().ok()) {
                Some(id) => id,
                None => return RpcResponse::invalid_params(id, "Missing or invalid entry_id"),
            };
            let mut q = queue.lock().unwrap();
            if q.cancel(entry_id) {
                RpcResponse::success(id, json!({ "ok": true }))
            } else {
                RpcResponse::error(id, -1, "Entry not found")
            }
        }

        Method::Ping => RpcResponse::success(id, json!({ "pong": true, "mode": "headless" })),

        _ => RpcResponse::error(
            id,
            -32601,
            format!("Method {:?} not available in headless mode", method),
        ),
    }
}
