use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum SandboxCommand {
    /// Show sandbox status for the active workspace.
    Status {
        /// Workspace ID (shows active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Enable sandbox for the active workspace.
    Enable {
        /// Workspace ID (targets active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Disable sandbox for the active workspace.
    Disable {
        /// Workspace ID (targets active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Add a path to the sandbox allowlist.
    Allow {
        /// Path to allow read-write access.
        path: String,
        /// Allow read-only access only.
        #[arg(long)]
        read_only: bool,
        /// Workspace ID (targets active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Add a path to the sandbox deny list.
    Deny {
        /// Path to deny access.
        path: String,
        /// Workspace ID (targets active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
}

impl SandboxCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::Status { id } => {
                let resp =
                    send_rpc(socket_path, "sandbox.status", json!({ "id": id })).await?;
                print_response(&resp)
            }
            Self::Enable { id } => {
                let resp = send_rpc(
                    socket_path,
                    "sandbox.enable",
                    json!({ "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Disable { id } => {
                let resp = send_rpc(
                    socket_path,
                    "sandbox.disable",
                    json!({ "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Allow {
                path,
                read_only,
                id,
            } => {
                let resp = send_rpc(
                    socket_path,
                    "sandbox.allow",
                    json!({ "path": path, "read_only": read_only, "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Deny { path, id } => {
                let resp = send_rpc(
                    socket_path,
                    "sandbox.deny",
                    json!({ "path": path, "id": id }),
                )
                .await?;
                print_response(&resp)
            }
        }
    }
}
