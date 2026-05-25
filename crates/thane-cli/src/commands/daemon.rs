//! `thane-cli daemon {start|stop|restart}` — manage the background daemon.
//!
//! These commands shell out to the `thane-daemon` binary on PATH; if it
//! cannot be found the user gets an actionable error.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum DaemonCommand {
    /// Spawn `thane-daemon` in the background.
    Start,
    /// Send SIGTERM to a running daemon.
    Stop,
    /// Stop the daemon (if running) and start a fresh one.
    Restart,
}

impl DaemonCommand {
    pub async fn execute(self, socket_path: &str) -> Result<()> {
        match self {
            Self::Start => start(socket_path),
            Self::Stop => stop(socket_path),
            Self::Restart => {
                stop(socket_path).ok();
                // Brief grace period for the previous daemon to release the socket.
                std::thread::sleep(std::time::Duration::from_millis(300));
                start(socket_path)
            }
        }
    }
}

fn start(socket_path: &str) -> Result<()> {
    if is_running(Path::new(socket_path)) {
        println!("daemon already running at {socket_path}");
        return Ok(());
    }
    let bin = find_daemon_binary().context(
        "thane-daemon not found on PATH; install it via `brew install thane` or place it on PATH",
    )?;
    // Spawn detached so the daemon survives this CLI exiting.
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("--socket").arg(socket_path);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        // Detach from the controlling terminal so closing the parent shell
        // doesn't terminate the daemon.
        cmd.pre_exec(|| {
            // setsid creates a new session and process group.
            if libc::setsid() == -1 {
                let e = std::io::Error::last_os_error();
                // ignore EPERM (already a session leader)
                if e.raw_os_error() != Some(libc::EPERM) {
                    return Err(e);
                }
            }
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .with_context(|| format!("spawning {}", bin.display()))?;
    println!("daemon started (pid {}) at {socket_path}", child.id());
    Ok(())
}

fn stop(socket_path: &str) -> Result<()> {
    // Read the daemon PID via the system.status RPC; SIGTERM it.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;

    let request = thane_rpc::protocol::RpcRequest::new("system.status", serde_json::json!({}));
    let resp = match runtime.block_on(thane_ipc::client::send_request(socket_path, &request)) {
        Ok(r) => r,
        Err(_) => {
            println!("daemon not running");
            return Ok(());
        }
    };
    let Some(result) = resp.result else {
        return Ok(());
    };
    let Some(pid) = result.get("daemon_pid").and_then(|v| v.as_u64()) else {
        anyhow::bail!("system.status response missing daemon_pid: {result}");
    };
    let pid = pid as i32;
    if pid <= 0 {
        anyhow::bail!("invalid pid in system.status: {pid}");
    }

    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        kill(Pid::from_raw(pid), Signal::SIGTERM)
            .with_context(|| format!("sending SIGTERM to pid {pid}"))?;
        println!("sent SIGTERM to daemon pid {pid}");
    }
    Ok(())
}

fn is_running(socket_path: &Path) -> bool {
    use std::os::unix::net::UnixStream;
    if !socket_path.exists() {
        return false;
    }
    UnixStream::connect(socket_path).is_ok()
}

/// Look for `thane-daemon` on PATH; fall back to common bundle locations.
fn find_daemon_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let candidate = PathBuf::from(dir).join("thane-daemon");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // macOS .app bundle layout.
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/thane.app/Contents/MacOS/thane-daemon",
            "/Applications/thane-macos.app/Contents/MacOS/thane-daemon",
        ];
        for c in candidates {
            if Path::new(c).is_file() {
                return Some(PathBuf::from(c));
            }
        }
    }
    None
}
