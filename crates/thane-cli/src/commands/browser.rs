use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum BrowserCommand {
    /// Open a URL in a new browser panel.
    Open {
        /// URL to open.
        url: String,
        /// Target workspace ID.
        #[arg(short, long)]
        workspace_id: Option<String>,
    },
    /// Navigate an existing browser panel to a URL.
    Navigate {
        /// Panel ID.
        panel_id: String,
        /// URL to navigate to.
        url: String,
    },
    /// Execute JavaScript in a browser panel.
    EvalJs {
        /// Panel ID.
        panel_id: String,
        /// JavaScript code to execute.
        script: String,
    },
    /// Get the accessibility tree of a browser panel.
    GetAccessibilityTree {
        /// Panel ID.
        panel_id: String,
    },
    /// Click an element in a browser panel.
    Click {
        /// Panel ID.
        panel_id: String,
        /// CSS selector of the element.
        selector: String,
    },
    /// Type text into an element in a browser panel.
    TypeText {
        /// Panel ID.
        panel_id: String,
        /// CSS selector of the element.
        selector: String,
        /// Text to type.
        text: String,
    },
    /// Take a screenshot of a browser panel (returns base64 PNG).
    Screenshot {
        /// Panel ID (uses focused browser if omitted).
        #[arg(short, long)]
        panel_id: Option<String>,
        /// Capture full document instead of visible viewport.
        #[arg(long)]
        full_page: bool,
    },
}

impl BrowserCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::Open { url, workspace_id } => {
                let resp = send_rpc(
                    socket_path,
                    "browser.open",
                    json!({ "url": url, "workspace_id": workspace_id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Navigate { panel_id, url } => {
                let resp = send_rpc(
                    socket_path,
                    "browser.navigate",
                    json!({ "panel_id": panel_id, "url": url }),
                )
                .await?;
                print_response(&resp)
            }
            Self::EvalJs { panel_id, script } => {
                let resp = send_rpc(
                    socket_path,
                    "browser.eval_js",
                    json!({ "panel_id": panel_id, "script": script }),
                )
                .await?;
                print_response(&resp)
            }
            Self::GetAccessibilityTree { panel_id } => {
                let resp = send_rpc(
                    socket_path,
                    "browser.get_accessibility_tree",
                    json!({ "panel_id": panel_id }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Click { panel_id, selector } => {
                let resp = send_rpc(
                    socket_path,
                    "browser.click_element",
                    json!({ "panel_id": panel_id, "selector": selector }),
                )
                .await?;
                print_response(&resp)
            }
            Self::TypeText {
                panel_id,
                selector,
                text,
            } => {
                let resp = send_rpc(
                    socket_path,
                    "browser.type_text",
                    json!({
                        "panel_id": panel_id,
                        "selector": selector,
                        "text": text,
                    }),
                )
                .await?;
                print_response(&resp)
            }
            Self::Screenshot {
                panel_id,
                full_page,
            } => {
                let mut params = json!({ "full_page": full_page });
                if let Some(ref pid) = panel_id {
                    params["panel_id"] = json!(pid);
                }
                let resp =
                    send_rpc(socket_path, "browser.screenshot", params).await?;
                print_response(&resp)
            }
        }
    }
}
