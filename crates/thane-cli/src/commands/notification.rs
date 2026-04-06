use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum NotificationCommand {
    /// List notifications.
    List {
        /// Limit the number of results.
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Send a notification to a workspace.
    Send {
        /// Notification title.
        title: String,
        /// Notification body.
        body: String,
        /// Target workspace ID.
        #[arg(short, long)]
        workspace_id: Option<String>,
    },
    /// Mark all notifications as read.
    MarkRead,
    /// Clear all notifications.
    Clear,
}

impl NotificationCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::List { limit } => {
                let resp = send_rpc(
                    socket_path,
                    "notification.list",
                    json!({ "limit": limit }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Send {
                title,
                body,
                workspace_id,
            } => {
                let resp = send_rpc(
                    socket_path,
                    "notification.send",
                    json!({
                        "title": title,
                        "body": body,
                        "workspace_id": workspace_id,
                    }),
                )
                .await?;
                print_response(&resp)
            }
            Self::MarkRead => {
                let resp = send_rpc(socket_path, "notification.mark_read", json!({})).await?;
                print_response(&resp)
            }
            Self::Clear => {
                let resp = send_rpc(socket_path, "notification.clear", json!({})).await?;
                print_response(&resp)
            }
        }
    }
}
