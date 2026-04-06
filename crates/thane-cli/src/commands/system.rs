use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use super::{print_response, send_rpc};

#[derive(Subcommand)]
pub enum SystemCommand {
    /// Get thane version.
    Version,
    /// Get current configuration.
    Config,
}

impl SystemCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::Version => {
                let resp = send_rpc(socket_path, "get_version", json!({})).await?;
                print_response(&resp)
            }
            Self::Config => {
                let resp = send_rpc(socket_path, "get_config", json!({})).await?;
                print_response(&resp)
            }
        }
    }
}

/// Send a ping to the thane socket.
pub async fn ping(socket_path: &str) -> Result<()> {
    let resp = send_rpc(socket_path, "ping", json!({})).await?;
    if resp.error.is_some() {
        print_response(&resp)?;
    } else {
        println!("pong");
    }
    Ok(())
}
