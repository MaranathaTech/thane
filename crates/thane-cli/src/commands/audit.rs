use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum AuditCommand {
    /// List recent audit events.
    List {
        /// Minimum severity filter (info, warning, alert, critical).
        #[arg(short, long)]
        severity: Option<String>,
        /// Maximum number of events to return.
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },
    /// Export all audit events as JSON.
    Export {
        /// Output file path (prints to stdout if not specified).
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Clear all audit events.
    Clear,
    /// Set the sensitive operation policy for a workspace.
    SetPolicy {
        /// Action to take: allow, warn, or block.
        action: String,
        /// Workspace ID (targets active if not specified).
        #[arg(short, long)]
        id: Option<String>,
    },
}

impl AuditCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::List { severity, limit } => {
                let resp = send_rpc(
                    socket_path,
                    "audit.list",
                    json!({ "severity": severity, "limit": limit }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Export { output } => {
                let resp = send_rpc(socket_path, "audit.export", json!({})).await?;
                if let Some(path) = output {
                    if let Some(ref result) = resp.result {
                        let json_str = serde_json::to_string_pretty(result)?;
                        std::fs::write(&path, json_str)?;
                        println!("Audit log exported to {path}");
                    }
                    Ok(())
                } else {
                    print_response(&resp)
                }
            }
            Self::Clear => {
                let resp = send_rpc(socket_path, "audit.clear", json!({})).await?;
                print_response(&resp)
            }
            Self::SetPolicy { action, id } => {
                let resp = send_rpc(
                    socket_path,
                    "audit.set_sensitive_policy",
                    json!({ "action": action, "id": id }),
                )
                .await?;
                print_response(&resp)
            }
        }
    }
}
