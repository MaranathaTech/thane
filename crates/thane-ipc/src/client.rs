use std::future::Future;
use std::pin::Pin;

use thane_rpc::protocol::{RpcRequest, RpcResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing;

/// A boxed future that resolves to an RpcResponse.
pub type RpcFuture = Pin<Box<dyn Future<Output = RpcResponse> + Send>>;

/// An async RPC handler: takes a request and returns a future resolving to a response.
pub type AsyncRpcHandler = Arc<dyn Fn(RpcRequest) -> RpcFuture + Send + Sync + 'static>;

use std::sync::Arc;

/// Handle a single client connection.
///
/// Reads newline-delimited JSON-RPC requests from the stream,
/// dispatches them via the provided async handler, and writes responses back.
pub async fn handle_client(
    stream: UnixStream,
    handler: AsyncRpcHandler,
) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let response = match serde_json::from_str::<RpcRequest>(trimmed) {
                    Ok(request) => {
                        tracing::debug!("RPC request: {}", request.method);

                        handler(request).await
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse RPC request: {e}");
                        RpcResponse::parse_error()
                    }
                };

                // Only send response if the request had an id (not a notification).
                if response.id.is_some() || response.error.is_some() {
                    let json = match serde_json::to_string(&response) {
                        Ok(j) => j,
                        Err(e) => {
                            tracing::error!("Failed to serialize RPC response: {e}");
                            continue;
                        }
                    };

                    if let Err(e) = writer.write_all(json.as_bytes()).await {
                        tracing::error!("Failed to write RPC response: {e}");
                        break;
                    }
                    if let Err(e) = writer.write_all(b"\n").await {
                        tracing::error!("Failed to write newline: {e}");
                        break;
                    }
                    if let Err(e) = writer.flush().await {
                        tracing::error!("Failed to flush: {e}");
                        break;
                    }
                }
            }
            Err(e) => {
                tracing::error!("Error reading from client: {e}");
                break;
            }
        }
    }

    tracing::debug!("Client disconnected");
}

/// Send a single RPC request to a running thane instance.
///
/// If the `THANE_TOKEN` environment variable is set, the token is sent
/// as the first line for authentication before the RPC request.
pub async fn send_request(
    socket_path: &str,
    request: &RpcRequest,
) -> Result<RpcResponse, Box<dyn std::error::Error>> {
    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();

    // If a token is set, send it as the first line for authentication.
    if let Ok(token) = std::env::var("THANE_TOKEN")
        && !token.is_empty()
    {
        writer.write_all(token.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        // Read the auth response.
        let mut auth_reader = BufReader::new(reader);
        let mut auth_line = String::new();
        auth_reader.read_line(&mut auth_line).await?;

        let auth_line = auth_line.trim();
        if auth_line.contains("\"denied\"") {
            return Err(format!("Authentication failed: {auth_line}").into());
        }

        // Auth succeeded — continue with the RPC request.
        let json = serde_json::to_string(request)?;
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        let mut line = String::new();
        auth_reader.read_line(&mut line).await?;

        let response: RpcResponse = serde_json::from_str(line.trim())?;
        return Ok(response);
    }

    // No token — send request directly (Open or Ancestry mode).
    let json = serde_json::to_string(request)?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;

    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let response: RpcResponse = serde_json::from_str(line.trim())?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Create a simple echo handler that returns the method name as the result.
    fn echo_handler() -> AsyncRpcHandler {
        Arc::new(|req: RpcRequest| {
            Box::pin(async move {
                RpcResponse::success(req.id, json!({"method": req.method}))
            })
        })
    }

    #[tokio::test]
    async fn test_handle_client_valid_request() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();
        let handler = echo_handler();

        let server_task = tokio::spawn(handle_client(server_stream, handler));

        let (reader, mut writer) = client_stream.into_split();
        let request = RpcRequest::new("ping", json!({}));
        let json = serde_json::to_string(&request).unwrap();
        writer.write_all(json.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let response: RpcResponse = serde_json::from_str(line.trim()).unwrap();

        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["method"], "ping");

        drop(writer);
        drop(reader);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_handle_client_invalid_json() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();
        let handler = echo_handler();

        let server_task = tokio::spawn(handle_client(server_stream, handler));

        let (reader, mut writer) = client_stream.into_split();
        writer.write_all(b"not valid json\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let response: RpcResponse = serde_json::from_str(line.trim()).unwrap();

        let err = response.error.unwrap();
        assert_eq!(err.code, -32700);

        drop(writer);
        drop(reader);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_handle_client_empty_lines_ignored() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();
        let handler = echo_handler();

        let server_task = tokio::spawn(handle_client(server_stream, handler));

        let (reader, mut writer) = client_stream.into_split();

        // Send empty lines, then a valid request.
        writer.write_all(b"\n\n\n").await.unwrap();
        let request = RpcRequest::new("test", json!({}));
        let json = serde_json::to_string(&request).unwrap();
        writer.write_all(json.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let response: RpcResponse = serde_json::from_str(line.trim()).unwrap();

        // Should only get one response (for the valid request, not for empty lines).
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["method"], "test");

        drop(writer);
        drop(reader);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_handle_client_multiple_requests() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();
        let handler = echo_handler();

        let server_task = tokio::spawn(handle_client(server_stream, handler));

        let (reader, mut writer) = client_stream.into_split();
        let mut reader = BufReader::new(reader);

        for method in &["method_a", "method_b", "method_c"] {
            let request = RpcRequest::new(*method, json!({}));
            let json = serde_json::to_string(&request).unwrap();
            writer.write_all(json.as_bytes()).await.unwrap();
            writer.write_all(b"\n").await.unwrap();
            writer.flush().await.unwrap();

            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let response: RpcResponse = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(response.result.unwrap()["method"], *method);
        }

        drop(writer);
        drop(reader);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_handle_client_eof_disconnects() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();
        let handler = echo_handler();

        let server_task = tokio::spawn(handle_client(server_stream, handler));

        // Drop the client immediately (EOF).
        drop(client_stream);

        // Server should return cleanly without panic.
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_handle_client_notification_no_response() {
        let (client_stream, server_stream) = UnixStream::pair().unwrap();

        // Handler returns a response with id: None (notification echo).
        let handler: AsyncRpcHandler = Arc::new(|req: RpcRequest| {
            Box::pin(async move {
                RpcResponse::success(req.id, json!({"notified": true}))
            })
        });

        let server_task = tokio::spawn(handle_client(server_stream, handler));

        let (reader, mut writer) = client_stream.into_split();

        // Send a notification (id: null).
        let notification = RpcRequest::notification("event.fired", json!({}));
        let json = serde_json::to_string(&notification).unwrap();
        writer.write_all(json.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        // Then send a real request to prove the server is still alive.
        let request = RpcRequest::new("ping", json!({}));
        let json = serde_json::to_string(&request).unwrap();
        writer.write_all(json.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let response: RpcResponse = serde_json::from_str(line.trim()).unwrap();

        // The only response we get should be for the ping request, not the notification.
        assert_eq!(response.id, Some(serde_json::Value::Number(1.into())));
        assert_eq!(response.result.unwrap()["notified"], true);

        drop(writer);
        drop(reader);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_send_request_roundtrip() {
        // Create a temp socket path.
        let socket_dir = std::env::temp_dir().join(format!("thane-test-{}", std::process::id()));
        std::fs::create_dir_all(&socket_dir).unwrap();
        let socket_path = socket_dir.join("test.sock");
        let socket_path_str = socket_path.to_str().unwrap().to_string();

        // Start a listener that accepts one connection and echoes back.
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let handler = echo_handler();
            handle_client(stream, handler).await;
        });

        // Ensure THANE_TOKEN is not set for this test.
        // We can't safely unset env vars in tests, so we rely on it not being set.
        // If it is set, the test still validates the token-auth path.

        let request = RpcRequest::new("workspace.list", json!({}));
        let response = send_request(&socket_path_str, &request).await.unwrap();

        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["method"], "workspace.list");

        // Clean up.
        server_task.await.unwrap();
        let _ = std::fs::remove_dir_all(&socket_dir);
    }
}
