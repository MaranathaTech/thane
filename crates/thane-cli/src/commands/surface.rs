use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum SurfaceCommand {
    /// Split the focused pane to the right.
    SplitRight,
    /// Split the focused pane downward.
    SplitDown,
    /// Close the focused pane.
    Close,
    /// Focus the next pane.
    FocusNext,
    /// Focus the previous pane.
    FocusPrev,
    /// Focus pane in a direction.
    Focus {
        /// Direction: up, down, left, right.
        direction: String,
    },
    /// Toggle zoom on the focused pane.
    ZoomToggle,
}

impl SurfaceCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::SplitRight => {
                let resp = send_rpc(socket_path, "surface.split_right", json!({})).await?;
                print_response(&resp)
            }
            Self::SplitDown => {
                let resp = send_rpc(socket_path, "surface.split_down", json!({})).await?;
                print_response(&resp)
            }
            Self::Close => {
                let resp = send_rpc(socket_path, "surface.close", json!({})).await?;
                print_response(&resp)
            }
            Self::FocusNext => {
                let resp = send_rpc(socket_path, "surface.focus_next", json!({})).await?;
                print_response(&resp)
            }
            Self::FocusPrev => {
                let resp = send_rpc(socket_path, "surface.focus_prev", json!({})).await?;
                print_response(&resp)
            }
            Self::Focus { direction } => {
                let resp = send_rpc(
                    socket_path,
                    "surface.focus_direction",
                    json!({ "direction": direction }),
                )
                .await?;
                print_response(&resp)
            }
            Self::ZoomToggle => {
                let resp = send_rpc(socket_path, "surface.zoom_toggle", json!({})).await?;
                print_response(&resp)
            }
        }
    }
}
