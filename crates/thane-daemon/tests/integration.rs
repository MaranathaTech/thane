//! Integration test for the daemon: spawn it as a subprocess, drive it
//! over the IPC socket, then shut it down cleanly.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::json;
use thane_ipc::client::send_request;
use thane_rpc::protocol::RpcRequest;

fn daemon_binary() -> PathBuf {
    // The harness sets `CARGO_BIN_EXE_thane-daemon` to the freshly-built bin.
    PathBuf::from(env!("CARGO_BIN_EXE_thane-daemon"))
}

/// Wait until a daemon is accepting connections on the socket, or timeout.
async fn wait_for_socket(socket: &std::path::Path, max: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < max {
        if socket.exists() {
            let req = RpcRequest::new("ping", json!({}));
            if send_request(socket.to_str().unwrap(), &req).await.is_ok() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn daemon_spawns_and_responds_to_status() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let socket = tmp.path().join("thane.sock");

    let mut child = std::process::Command::new(daemon_binary())
        .arg("--socket")
        .arg(&socket)
        // Bypass the platform secret store: keychain access can prompt for user
        // approval on macOS, which deadlocks headless test runs.
        .env("THANE_AUDIT_HMAC_KEY_HEX", "0".repeat(64))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn daemon");

    assert!(
        wait_for_socket(&socket, Duration::from_secs(5)).await,
        "daemon did not start within 5s"
    );

    // system.status must report we're running.
    let resp = send_request(
        socket.to_str().unwrap(),
        &RpcRequest::new("system.status", json!({})),
    )
    .await
    .expect("send system.status");
    assert!(resp.error.is_none(), "system.status returned error: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["daemon_running"], true);
    assert!(result["daemon_pid"].as_u64().unwrap() > 0);

    // SIGTERM the daemon and let it shut down.
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let _ = child.wait();
    // After shutdown the socket file should be gone (best-effort cleanup).
    // We don't assert because a race with abort() in the daemon's select! arm
    // can leave the file briefly; the important property is the daemon exited.
}

#[tokio::test]
async fn daemon_round_trips_a_queue_submission() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let socket = tmp.path().join("thane.sock");

    let mut child = std::process::Command::new(daemon_binary())
        .arg("--socket")
        .arg(&socket)
        .env("THANE_AUDIT_HMAC_KEY_HEX", "0".repeat(64))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn daemon");

    assert!(wait_for_socket(&socket, Duration::from_secs(5)).await);

    // Submit a task — content is opaque to the daemon's RPC; the executor
    // would try to spawn claude, but for this test we only care that the
    // RPC round-trips end-to-end and the queue state updates.
    let submit = send_request(
        socket.to_str().unwrap(),
        &RpcRequest::new(
            "agent_queue.submit",
            json!({"content": "noop", "priority": 0}),
        ),
    )
    .await
    .expect("submit");
    assert!(submit.error.is_none(), "submit returned error: {:?}", submit.error);
    let entry_id = submit.result.unwrap()["entry_id"].as_str().unwrap().to_string();

    // list should show it.
    let list = send_request(
        socket.to_str().unwrap(),
        &RpcRequest::new("agent_queue.list", json!({})),
    )
    .await
    .expect("list");
    let entries = list.result.unwrap()["entries"].as_array().unwrap().clone();
    assert!(entries.iter().any(|e| e["id"].as_str() == Some(&entry_id)));

    // Cancel so the executor doesn't keep trying to spawn claude.
    let cancel = send_request(
        socket.to_str().unwrap(),
        &RpcRequest::new("agent_queue.cancel", json!({"entry_id": entry_id})),
    )
    .await
    .expect("cancel");
    assert!(cancel.error.is_none(), "cancel returned error: {:?}", cancel.error);

    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let _ = child.wait();
}

#[test]
fn launchd_plist_is_valid_xml() {
    // The macOS launchctl tool ships with plutil; if it's available, lint
    // the plist we'd write to disk.
    #[cfg(target_os = "macos")]
    {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bin = tmp.path().join("thane-daemon");
        std::fs::write(&bin, "stub").unwrap();
        let plist_path = tmp.path().join("com.thane.daemon.plist");
        let contents = thane_daemon::launchd::plist_contents(&bin);
        std::fs::write(&plist_path, contents).unwrap();
        let output = std::process::Command::new("plutil")
            .arg("-lint")
            .arg(&plist_path)
            .output();
        if let Ok(out) = output {
            assert!(
                out.status.success(),
                "plutil rejected plist: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    // No-op on non-macOS.
}

#[test]
fn systemd_unit_parses() {
    #[cfg(target_os = "linux")]
    {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bin = tmp.path().join("thane-daemon");
        std::fs::write(&bin, "stub").unwrap();
        let unit_path = tmp.path().join("thane-daemon.service");
        std::fs::write(&unit_path, thane_daemon::systemd::unit_contents(&bin)).unwrap();

        // Best-effort: only verify if systemd-analyze is on PATH.
        if let Ok(out) = std::process::Command::new("systemd-analyze")
            .arg("verify")
            .arg(&unit_path)
            .output()
        {
            assert!(
                out.status.success(),
                "systemd-analyze rejected unit: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    // No-op on non-Linux.
}
