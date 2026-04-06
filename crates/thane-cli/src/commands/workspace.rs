use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// List all workspaces.
    List,
    /// Create a new workspace.
    Create {
        /// Workspace title.
        #[arg(short, long)]
        title: Option<String>,
        /// Working directory.
        #[arg(short, long)]
        cwd: Option<String>,
    },
    /// Select a workspace by index.
    Select {
        /// Workspace index (0-based).
        index: usize,
    },
    /// Close a workspace.
    Close {
        /// Workspace ID (closes active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Rename a workspace.
    Rename {
        /// New title.
        title: String,
        /// Workspace ID (renames active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Get workspace info.
    Info {
        /// Workspace ID (shows active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
    /// List recently closed workspaces.
    History,
    /// Reopen a recently closed workspace.
    Reopen {
        /// Original workspace ID.
        id: String,
    },
    /// Clear the recently closed workspace history.
    HistoryClear,
}

impl WorkspaceCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::List => {
                let resp = send_rpc(socket_path, "workspace.list", json!({})).await?;
                print_response(&resp)
            }
            Self::Create { title, cwd } => {
                let resp = send_rpc(
                    socket_path,
                    "workspace.create",
                    json!({ "title": title, "cwd": cwd }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Select { index } => {
                let resp = send_rpc(
                    socket_path,
                    "workspace.select",
                    json!({ "index": index }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Close { id } => {
                let resp = send_rpc(
                    socket_path,
                    "workspace.close",
                    json!({ "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Rename { title, id } => {
                let resp = send_rpc(
                    socket_path,
                    "workspace.rename",
                    json!({ "title": title, "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Info { id } => {
                let resp = send_rpc(
                    socket_path,
                    "workspace.get_info",
                    json!({ "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::History => {
                let resp = send_rpc(socket_path, "workspace.history_list", json!({})).await?;
                print_response(&resp)
            }
            Self::Reopen { id } => {
                let resp = send_rpc(
                    socket_path,
                    "workspace.history_reopen",
                    json!({ "id": id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::HistoryClear => {
                let resp =
                    send_rpc(socket_path, "workspace.history_clear", json!({})).await?;
                print_response(&resp)
            }
        }
    }
}
