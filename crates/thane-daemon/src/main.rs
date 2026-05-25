//! `thane-daemon` — headless daemon entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use thane_daemon::default_socket_path;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "thane-daemon", version, about = "thane background daemon")]
struct Cli {
    /// Override the socket path (defaults to the platform-canonical location).
    #[arg(long, env = "THANE_SOCKET_PATH")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Install and start the LaunchAgent (macOS) so the daemon runs at login.
    #[cfg(target_os = "macos")]
    InstallLaunchAgent,
    /// Stop and remove the LaunchAgent (macOS).
    #[cfg(target_os = "macos")]
    UninstallLaunchAgent,
    /// Install and start the systemd user service (Linux) so the daemon
    /// runs at login.
    #[cfg(target_os = "linux")]
    InstallUserService,
    /// Stop and remove the systemd user service (Linux).
    #[cfg(target_os = "linux")]
    UninstallUserService,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(default_socket_path);

    let result = match cli.command {
        Some(command) => run_command(command).await,
        None => thane_daemon::run(socket).await,
    };

    if let Err(e) = result {
        eprintln!("thane-daemon: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run_command(command: Command) -> Result<()> {
    match command {
        #[cfg(target_os = "macos")]
        Command::InstallLaunchAgent => {
            let path = thane_daemon::launchd::install()?;
            println!("Installed LaunchAgent: {}", path.display());
        }
        #[cfg(target_os = "macos")]
        Command::UninstallLaunchAgent => {
            thane_daemon::launchd::uninstall()?;
            println!("LaunchAgent removed");
        }
        #[cfg(target_os = "linux")]
        Command::InstallUserService => {
            let path = thane_daemon::systemd::install()?;
            println!("Installed systemd user service: {}", path.display());
        }
        #[cfg(target_os = "linux")]
        Command::UninstallUserService => {
            thane_daemon::systemd::uninstall()?;
            println!("systemd user service removed");
        }
    }
    Ok(())
}
