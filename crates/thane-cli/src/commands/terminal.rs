use anyhow::Result;
use base64::Engine;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum TerminalCommand {
    /// Take a screenshot of a terminal panel (returns base64 PNG).
    Screenshot {
        /// Panel ID (uses focused terminal if omitted).
        #[arg(short, long)]
        panel_id: Option<String>,
        /// Write raw PNG to this file path instead of printing JSON.
        #[arg(short, long)]
        output: Option<String>,
    },
}

impl TerminalCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::Screenshot { panel_id, output } => {
                let mut params = json!({});
                if let Some(ref pid) = panel_id {
                    params["panel_id"] = json!(pid);
                }
                let resp =
                    send_rpc(socket_path, "terminal.screenshot", params).await?;

                if let Some(ref path) = output {
                    if let Some(ref error) = resp.error {
                        eprintln!("Error {}: {}", error.code, error.message);
                        std::process::exit(1);
                    }
                    if let Some(ref result) = resp.result {
                        let b64 = result["image"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("Missing image in response"))?;
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(b64)
                            .map_err(|e| anyhow::anyhow!("Base64 decode error: {e}"))?;
                        std::fs::write(path, &bytes)?;
                        eprintln!("Screenshot saved to {path}");
                    }
                    Ok(())
                } else {
                    print_response(&resp)
                }
            }
        }
    }
}
