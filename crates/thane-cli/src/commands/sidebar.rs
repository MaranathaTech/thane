use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum SidebarCommand {
    /// Set a status entry in the sidebar.
    SetStatus {
        /// Status label.
        label: String,
        /// Status value.
        value: String,
        /// Style: normal, success, warning, error, muted.
        #[arg(short, long, default_value = "normal")]
        style: String,
        /// Target workspace ID.
        #[arg(short, long)]
        workspace_id: Option<String>,
    },
    /// Get sidebar metadata for a workspace.
    GetMetadata {
        /// Target workspace ID.
        #[arg(short, long)]
        workspace_id: Option<String>,
    },
}

impl SidebarCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::SetStatus {
                label,
                value,
                style,
                workspace_id,
            } => {
                let resp = send_rpc(
                    socket_path,
                    "sidebar.set_status",
                    json!({
                        "label": label,
                        "value": value,
                        "style": style,
                        "workspace_id": workspace_id,
                    }),
                )
                .await?;
                print_response(&resp)
            }
            Self::GetMetadata { workspace_id } => {
                let resp = send_rpc(
                    socket_path,
                    "sidebar.get_metadata",
                    json!({ "workspace_id": workspace_id }),
                )
                .await?;
                print_response(&resp)
            }
        }
    }
}
