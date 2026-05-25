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
    /// Print daemon health (daemon_running, pid, socket, uptime, version).
    Status,
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
            Self::Status => status(socket_path).await,
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

/// Print the daemon status. When the daemon is not running, exit non-zero
/// with an actionable hint so scripts can detect the condition.
pub async fn status(socket_path: &str) -> Result<()> {
    match send_rpc(socket_path, "system.status", json!({})).await {
        Ok(resp) => print_response(&resp),
        Err(_) => {
            eprintln!("Daemon not running (failed to connect to {socket_path}).");
            eprintln!();
            #[cfg(target_os = "macos")]
            eprintln!(
                "  Start at login:   thane-daemon install-launch-agent"
            );
            #[cfg(target_os = "linux")]
            eprintln!(
                "  Start at login:   thane-daemon install-user-service"
            );
            eprintln!("  Start manually:   thane-daemon &");
            std::process::exit(1);
        }
    }
}
