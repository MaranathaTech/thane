//! thane-daemon: headless background process that owns the IPC socket and
//! drives the agent queue when the GUI app is not running.
//!
//! The daemon exists so that `thane-cli` works (and queued tasks execute)
//! immediately after install and at every login, without requiring the user
//! to open the GUI first.

pub mod executor;
pub mod rpc;
pub mod state;

#[cfg(target_os = "macos")]
pub mod launchd;

#[cfg(target_os = "linux")]
pub mod systemd;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use thane_platform::traits::PlatformDirs;
use tokio::signal::unix::{SignalKind, signal};

pub use state::DaemonState;

/// Get the platform-specific directory provider.
#[cfg(target_os = "linux")]
pub fn platform_dirs() -> thane_platform::LinuxDirs {
    thane_platform::LinuxDirs
}

#[cfg(target_os = "macos")]
pub fn platform_dirs() -> thane_platform::MacosDirs {
    thane_platform::MacosDirs
}

/// Resolve the canonical Unix socket path used by the daemon and CLI.
pub fn default_socket_path() -> PathBuf {
    platform_dirs().socket_path()
}

/// Run the daemon to completion.
///
/// Starts the IPC server, the queue executor, and periodic auto-save /
/// config-reload / audit-flush loops. Blocks until SIGTERM or SIGINT.
pub async fn run(socket_path: PathBuf) -> Result<()> {
    let dirs = platform_dirs();
    dirs.ensure_dirs()
        .context("creating platform directories")?;

    // Ensure socket parent dir exists.
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating socket parent dir {}", parent.display()))?;
    }

    let state = Arc::new(DaemonState::new(Instant::now(), socket_path.clone())?);

    // Bind the IPC server first so any caller that races us sees the socket.
    let handler = rpc::build_handler(state.clone());
    let server_socket = socket_path.clone();
    let server_handle = tokio::spawn(async move {
        if let Err(e) = thane_ipc::server::start_server(
            &server_socket,
            handler,
            thane_ipc::auth::AccessMode::Open,
        )
        .await
        {
            tracing::error!("IPC server error: {e}");
        }
    });

    tracing::info!("daemon listening on {}", socket_path.display());

    // Queue executor loop.
    let exec_state = state.clone();
    let exec_handle = tokio::spawn(async move {
        executor::run_loop(exec_state).await;
    });

    // Periodic tasks: config reload (5s), audit flush (10s), queue auto-save (8s).
    let cfg_state = state.clone();
    let cfg_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.tick().await; // First tick is immediate; skip.
        loop {
            interval.tick().await;
            cfg_state.reload_config_if_changed();
        }
    });

    let audit_state = state.clone();
    let audit_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        interval.tick().await;
        loop {
            interval.tick().await;
            audit_state.flush_audit();
        }
    });

    let save_state = state.clone();
    let save_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(8));
        interval.tick().await;
        loop {
            interval.tick().await;
            save_state.save_queue();
        }
    });

    // Wait for SIGTERM / SIGINT.
    let mut sigterm = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    tokio::select! {
        _ = sigterm.recv() => tracing::info!("received SIGTERM, shutting down"),
        _ = sigint.recv() => tracing::info!("received SIGINT, shutting down"),
    }

    // Best-effort cleanup.
    state.flush_audit();
    state.save_queue();
    thane_ipc::server::cleanup_socket(&socket_path);

    // Abort background tasks.
    server_handle.abort();
    exec_handle.abort();
    cfg_handle.abort();
    audit_handle.abort();
    save_handle.abort();

    Ok(())
}

/// Whether a daemon process is currently listening on the canonical socket.
///
/// Performs a non-blocking connect — returns true if the socket accepts
/// the connection, false otherwise.
pub fn is_running(socket_path: &std::path::Path) -> bool {
    use std::os::unix::net::UnixStream;
    if !socket_path.exists() {
        return false;
    }
    UnixStream::connect(socket_path).is_ok()
}
