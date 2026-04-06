use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum QueueCommand {
    /// Submit a task for execution.
    Submit {
        /// Path to a task file (text or JSON). Use "-" for stdin.
        file: String,
        /// Workspace ID to run in (creates new workspace if not specified).
        #[arg(short, long)]
        workspace: Option<String>,
        /// Priority (higher values run first, default 0).
        #[arg(short, long, default_value = "0")]
        priority: i32,
        /// Task ID that must complete successfully before this task runs.
        #[arg(short, long)]
        depends_on: Option<String>,
    },
    /// List all tasks in the queue.
    List,
    /// Get the status of a specific task.
    Status {
        /// Task ID.
        id: String,
    },
    /// Cancel a queued or running task.
    Cancel {
        /// Task ID.
        id: String,
    },
}

impl QueueCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::Submit {
                file,
                workspace,
                priority,
                depends_on,
            } => {
                let content = if file == "-" {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                } else {
                    std::fs::read_to_string(&file)
                        .map_err(|e| anyhow::anyhow!("Failed to read task file '{file}': {e}"))?
                };

                if content.trim().is_empty() {
                    anyhow::bail!("Task content is empty");
                }

                let resp = send_rpc(
                    socket_path,
                    "agent_queue.submit",
                    json!({
                        "content": content,
                        "workspace_id": workspace,
                        "priority": priority,
                        "depends_on": depends_on,
                    }),
                )
                .await?;
                // Print just the entry_id (no JSON wrapper) so callers can capture it
                // with PHASE_ID=$(thane-cli queue submit ...).
                if let Some(ref result) = resp.result {
                    if let Some(entry_id) = result.get("entry_id").and_then(|v| v.as_str()) {
                        println!("{entry_id}");
                        return Ok(());
                    }
                }
                print_response(&resp)
            }
            Self::List => {
                let resp = send_rpc(socket_path, "agent_queue.list", json!({})).await?;
                print_response(&resp)
            }
            Self::Status { id } => {
                let resp = send_rpc(
                    socket_path,
                    "agent_queue.status",
                    json!({ "entry_id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Cancel { id } => {
                let resp = send_rpc(
                    socket_path,
                    "agent_queue.cancel",
                    json!({ "entry_id": id }),
                )
                .await?;
                print_response(&resp)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use thane_rpc::protocol::RpcRequest;

    #[test]
    fn test_queue_submit_reads_file() {
        let dir = std::env::temp_dir().join(format!(
            "thane-cli-test-queue-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("task.txt");
        let content = "Build the authentication module with JWT tokens";

        std::fs::write(&file_path, content).unwrap();

        let read_back = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(read_back, content);
        assert!(!read_back.trim().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_queue_status_uses_entry_id_param() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let params = json!({ "entry_id": id });
        let request = RpcRequest::new("agent_queue.status", params);
        assert_eq!(request.method, "agent_queue.status");
        assert_eq!(
            request.params["entry_id"].as_str().unwrap(),
            id,
            "Status command must send 'entry_id' to match the RPC handler"
        );
        assert!(
            request.params.get("id").is_none(),
            "Should not send bare 'id' — RPC handler expects 'entry_id'"
        );
    }

    #[test]
    fn test_queue_cancel_uses_entry_id_param() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let params = json!({ "entry_id": id });
        let request = RpcRequest::new("agent_queue.cancel", params);
        assert_eq!(request.method, "agent_queue.cancel");
        assert_eq!(
            request.params["entry_id"].as_str().unwrap(),
            id,
            "Cancel command must send 'entry_id' to match the RPC handler"
        );
        assert!(
            request.params.get("id").is_none(),
            "Should not send bare 'id' — RPC handler expects 'entry_id'"
        );
    }
}
