pub mod audit;
pub mod browser;
pub mod notification;
pub mod queue;
pub mod sandbox;
pub mod sidebar;
pub mod surface;
pub mod system;
pub mod terminal;
pub mod workspace;

use thane_rpc::protocol::{RpcRequest, RpcResponse};
use anyhow::Result;
use serde_json::Value;

/// Send an RPC request to the thane socket and return the response.
pub async fn send_rpc(
    socket_path: &str,
    method: &str,
    params: Value,
) -> Result<RpcResponse> {
    let request = RpcRequest::new(method, params);
    let response = thane_ipc::client::send_request(socket_path, &request).await
        .map_err(|e| anyhow::anyhow!("Failed to connect to thane: {e}"))?;
    Ok(response)
}

/// Print an RPC response as JSON.
pub fn print_response(response: &RpcResponse) -> Result<()> {
    if let Some(ref error) = response.error {
        eprintln!("Error {}: {}", error.code, error.message);
        std::process::exit(1);
    }
    if let Some(ref result) = response.result {
        println!("{}", serde_json::to_string_pretty(result)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_print_response_success() {
        let response = RpcResponse::success(
            Some(Value::Number(1.into())),
            json!({"key": "val"}),
        );
        // Should not panic or error.
        let result = print_response(&response);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_response_no_result() {
        let response = RpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: None,
            id: Some(Value::Number(1.into())),
        };
        let result = print_response(&response);
        assert!(result.is_ok());
    }

    #[test]
    fn test_send_rpc_builds_correct_request() {
        // Verify RpcRequest::new produces the correct structure.
        let request = RpcRequest::new("workspace.create", json!({"title": "test"}));
        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.method, "workspace.create");
        assert_eq!(request.params["title"], "test");
        assert_eq!(request.id, Some(Value::Number(1.into())));
    }
}
