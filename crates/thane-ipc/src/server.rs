use std::path::Path;
use std::sync::Arc;

use tokio::net::UnixListener;
use tracing;

use crate::auth::{AccessMode, verify_access};
use crate::client::{AsyncRpcHandler, handle_client};

/// Start the IPC socket server.
///
/// Listens on a Unix domain socket and dispatches incoming JSON-RPC
/// requests via the provided handler function.
///
/// `access_mode` controls how connecting clients are authenticated:
/// - `Open`: any client may connect (development only)
/// - `Ancestry`: client must be a child process of the thane instance
/// - `Token(secret)`: client must send the token as the first line
///
/// The handler runs in the tokio runtime. For GTK operations,
/// the handler should send messages to the glib main loop via a channel.
pub async fn start_server(
    socket_path: &Path,
    handler: AsyncRpcHandler,
    access_mode: AccessMode,
) -> Result<(), Box<dyn std::error::Error>> {
    // Remove stale socket file if it exists.
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    // Ensure parent directory exists.
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    tracing::info!("IPC server listening on {}", socket_path.display());

    // Set socket permissions (owner-only).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(socket_path, perms)?;
    }

    let server_pid = std::process::id();
    let access_mode = Arc::new(access_mode);

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                tracing::debug!("New IPC client connected");

                // Extract peer PID from Unix socket credentials.
                let client_pid = get_peer_pid(&stream);

                let mode = Arc::clone(&access_mode);
                let handler = Arc::clone(&handler);

                tokio::spawn(async move {
                    // For Ancestry mode, verify immediately (no handshake needed).
                    // For Token mode, the first line from the client is the token
                    // (handled inside handle_client_authenticated).
                    match mode.as_ref() {
                        AccessMode::Open => {
                            tracing::debug!("Access mode: open, allowing connection");
                            handle_client(stream, handler).await;
                        }
                        AccessMode::Ancestry => {
                            match verify_access(&mode, client_pid, server_pid, None) {
                                Ok(()) => {
                                    tracing::debug!(
                                        "Ancestry check passed for PID {:?}",
                                        client_pid
                                    );
                                    handle_client(stream, handler).await;
                                }
                                Err(e) => {
                                    tracing::warn!("Access denied: {e}");
                                    reject_client(stream, &e.to_string()).await;
                                }
                            }
                        }
                        AccessMode::Token(_) => {
                            handle_client_with_token_auth(stream, &mode, client_pid, server_pid, handler).await;
                        }
                    }
                });
            }
            Err(e) => {
                tracing::error!("Error accepting IPC connection: {e}");
            }
        }
    }
}

/// Handle a client that must authenticate with a token as its first line.
async fn handle_client_with_token_auth(
    stream: tokio::net::UnixStream,
    mode: &AccessMode,
    client_pid: Option<u32>,
    server_pid: u32,
    handler: AsyncRpcHandler,
) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Read the first line as the auth token.
    let mut token_line = String::new();
    match reader.read_line(&mut token_line).await {
        Ok(0) => {
            tracing::warn!("Token auth: client disconnected before sending token");
            return;
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("Token auth: failed to read token line: {e}");
            return;
        }
    }

    let provided_token = token_line.trim();
    match verify_access(mode, client_pid, server_pid, Some(provided_token)) {
        Ok(()) => {
            tracing::debug!("Token auth passed for PID {:?}", client_pid);
            // Send an OK acknowledgment so the client knows auth succeeded.
            if let Err(e) = writer.write_all(b"{\"auth\":\"ok\"}\n").await {
                tracing::error!("Failed to send auth OK: {e}");
                return;
            }
            let _ = writer.flush().await;

            // Reunite the split stream and continue with normal handling.
            let stream = reader.into_inner().reunite(writer).expect("reunite failed");
            handle_client(stream, handler).await;
        }
        Err(e) => {
            tracing::warn!("Token auth failed: {e}");
            let error_json = format!("{{\"auth\":\"denied\",\"error\":\"{e}\"}}\n");
            let _ = writer.write_all(error_json.as_bytes()).await;
            let _ = writer.flush().await;
        }
    }
}

/// Send an access-denied error to a client and close the connection.
async fn reject_client(stream: tokio::net::UnixStream, reason: &str) {
    use tokio::io::AsyncWriteExt;

    let (_, mut writer) = stream.into_split();
    let error_json = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{{\"code\":-32600,\"message\":\"{reason}\"}}}}\n"
    );
    let _ = writer.write_all(error_json.as_bytes()).await;
    let _ = writer.flush().await;
}

/// Extract the peer PID from a Unix domain socket using SO_PEERCRED (Linux)
/// or LOCAL_PEERPID (macOS).
#[cfg(target_os = "linux")]
fn get_peer_pid(stream: &tokio::net::UnixStream) -> Option<u32> {
    use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};

    match getsockopt(stream, PeerCredentials) {
        Ok(cred) => {
            let pid = cred.pid() as u32;
            tracing::trace!("Peer credentials: PID={pid}, UID={}", cred.uid());
            Some(pid)
        }
        Err(e) => {
            tracing::warn!("Failed to get peer credentials: {e}");
            None
        }
    }
}

/// Extract the peer PID from a Unix domain socket using LOCAL_PEERPID (macOS).
#[cfg(target_os = "macos")]
fn get_peer_pid(stream: &tokio::net::UnixStream) -> Option<u32> {
    use nix::sys::socket::{getsockopt, sockopt::LocalPeerPid};

    match getsockopt(stream, LocalPeerPid) {
        Ok(pid) => {
            let pid = pid as u32;
            tracing::trace!("Peer PID={pid}");
            Some(pid)
        }
        Err(e) => {
            tracing::warn!("Failed to get peer PID: {e}");
            None
        }
    }
}

/// Cleanup the socket file on shutdown.
pub fn cleanup_socket(socket_path: &Path) {
    if socket_path.exists()
        && let Err(e) = std::fs::remove_file(socket_path) {
            tracing::warn!("Failed to remove socket file: {e}");
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{AsyncRpcHandler, RpcFuture};
    use serde_json::json;
    use thane_rpc::protocol::{RpcRequest, RpcResponse};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// Create a simple handler that returns `{"ok": true}`.
    fn ok_handler() -> AsyncRpcHandler {
        Arc::new(|req: RpcRequest| -> RpcFuture {
            Box::pin(async move {
                RpcResponse::success(req.id, json!({"ok": true}))
            })
        })
    }

    /// Create a unique temp directory for socket tests.
    fn test_socket_dir(test_name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "thane-ipc-test-{}-{test_name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_cleanup_socket_existing() {
        let dir = test_socket_dir("cleanup-existing");
        let sock = dir.join("test.sock");
        std::fs::write(&sock, "").unwrap();
        assert!(sock.exists());

        cleanup_socket(&sock);
        assert!(!sock.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cleanup_socket_missing() {
        let dir = test_socket_dir("cleanup-missing");
        let sock = dir.join("nonexistent.sock");

        // Should not panic.
        cleanup_socket(&sock);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Spawn `start_server` in a background task, mapping the error to a Send type.
    fn spawn_server(
        sock: std::path::PathBuf,
        handler: AsyncRpcHandler,
        mode: AccessMode,
    ) -> tokio::task::JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            start_server(&sock, handler, mode)
                .await
                .map_err(|e| e.to_string())
        })
    }

    #[tokio::test]
    async fn test_server_removes_stale_socket() {
        let dir = test_socket_dir("stale-socket");
        let sock = dir.join("stale.sock");

        // Pre-create a stale file at the socket path.
        std::fs::write(&sock, "stale").unwrap();
        assert!(sock.exists());

        let handler = ok_handler();
        let server_task = spawn_server(sock.clone(), handler, AccessMode::Open);

        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Verify we can connect (the stale file was replaced with a real socket).
        let stream = tokio::net::UnixStream::connect(&sock).await;
        assert!(stream.is_ok(), "Should be able to connect to the server");

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_server_creates_socket_with_permissions() {
        let dir = test_socket_dir("perms");
        let sock = dir.join("perms.sock");

        let handler = ok_handler();
        let server_task = spawn_server(sock.clone(), handler, AccessMode::Open);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Check permissions.
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&sock).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "Socket should have 0o700 permissions");

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_server_open_mode_accepts() {
        let dir = test_socket_dir("open-mode");
        let sock = dir.join("open.sock");

        let handler = ok_handler();
        let server_task = spawn_server(sock.clone(), handler, AccessMode::Open);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect and send a request.
        let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let (reader, mut writer) = stream.into_split();

        let request = RpcRequest::new("test.method", json!({}));
        let json_str = serde_json::to_string(&request).unwrap();
        writer.write_all(json_str.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let response: RpcResponse = serde_json::from_str(line.trim()).unwrap();

        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["ok"], true);

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_server_token_mode_valid_token() {
        let dir = test_socket_dir("token-valid");
        let sock = dir.join("token.sock");
        let secret = "test-secret-42";

        let handler = ok_handler();
        let mode = AccessMode::Token(secret.to_string());
        let server_task = spawn_server(sock.clone(), handler, mode);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect and authenticate.
        let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let (reader, mut writer) = stream.into_split();

        // Send token as first line.
        writer.write_all(secret.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut auth_line = String::new();
        reader.read_line(&mut auth_line).await.unwrap();
        assert!(
            auth_line.contains("\"ok\""),
            "Expected auth OK, got: {auth_line}"
        );

        // Now send a real RPC request.
        let request = RpcRequest::new("test.method", json!({}));
        let json_str = serde_json::to_string(&request).unwrap();
        writer.write_all(json_str.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut rpc_line = String::new();
        reader.read_line(&mut rpc_line).await.unwrap();
        let response: RpcResponse = serde_json::from_str(rpc_line.trim()).unwrap();
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["ok"], true);

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_server_token_mode_invalid_token() {
        let dir = test_socket_dir("token-invalid");
        let sock = dir.join("token.sock");
        let secret = "correct-secret";

        let handler = ok_handler();
        let mode = AccessMode::Token(secret.to_string());
        let server_task = spawn_server(sock.clone(), handler, mode);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Connect with wrong token.
        let stream = tokio::net::UnixStream::connect(&sock).await.unwrap();
        let (reader, mut writer) = stream.into_split();

        writer.write_all(b"wrong-secret\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut auth_line = String::new();
        reader.read_line(&mut auth_line).await.unwrap();
        assert!(
            auth_line.contains("\"denied\""),
            "Expected auth denied, got: {auth_line}"
        );

        server_task.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
