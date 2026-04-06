use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub id: Option<Value>,
}

impl RpcRequest {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id: Some(Value::Number(1.into())),
        }
    }

    pub fn notification(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id: None,
        }
    }
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    pub id: Option<Value>,
}

impl RpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }

    pub fn parse_error() -> Self {
        Self::error(None, -32700, "Parse error")
    }

    pub fn invalid_request(id: Option<Value>) -> Self {
        Self::error(id, -32600, "Invalid Request")
    }

    pub fn method_not_found(id: Option<Value>, method: &str) -> Self {
        Self::error(id, -32601, format!("Method not found: {method}"))
    }

    pub fn invalid_params(id: Option<Value>, msg: impl Into<String>) -> Self {
        Self::error(id, -32602, msg)
    }

    pub fn internal_error(id: Option<Value>, msg: impl Into<String>) -> Self {
        Self::error(id, -32603, msg)
    }
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_new() {
        let req = RpcRequest::new("ping", serde_json::json!({}));
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "ping");
        assert_eq!(req.params, serde_json::json!({}));
        assert_eq!(req.id, Some(Value::Number(1.into())));
    }

    #[test]
    fn test_request_notification() {
        let req = RpcRequest::notification("workspace.list", serde_json::json!(null));
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "workspace.list");
        assert!(req.id.is_none());
    }

    #[test]
    fn test_response_success() {
        let resp = RpcResponse::success(Some(Value::Number(1.into())), serde_json::json!("ok"));
        assert_eq!(resp.jsonrpc, "2.0");
        assert_eq!(resp.result, Some(serde_json::json!("ok")));
        assert!(resp.error.is_none());
        assert_eq!(resp.id, Some(Value::Number(1.into())));
    }

    #[test]
    fn test_response_error() {
        let resp = RpcResponse::error(Some(Value::Number(1.into())), -32000, "custom error");
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32000);
        assert_eq!(err.message, "custom error");
    }

    #[test]
    fn test_parse_error() {
        let resp = RpcResponse::parse_error();
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32700);
        assert!(resp.id.is_none());
    }

    #[test]
    fn test_invalid_request() {
        let resp = RpcResponse::invalid_request(Some(Value::Number(5.into())));
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(resp.id, Some(Value::Number(5.into())));
    }

    #[test]
    fn test_method_not_found() {
        let resp = RpcResponse::method_not_found(None, "foo.bar");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("foo.bar"));
    }

    #[test]
    fn test_invalid_params() {
        let resp = RpcResponse::invalid_params(None, "missing field");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "missing field");
    }

    #[test]
    fn test_internal_error() {
        let resp = RpcResponse::internal_error(None, "something broke");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32603);
    }

    #[test]
    fn test_request_roundtrip() {
        let req = RpcRequest::new("workspace.create", serde_json::json!({"title": "test"}));
        let json = serde_json::to_string(&req).unwrap();
        let parsed: RpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.jsonrpc, "2.0");
        assert_eq!(parsed.method, "workspace.create");
        assert_eq!(parsed.params["title"], "test");
        assert_eq!(parsed.id, Some(Value::Number(1.into())));
    }

    #[test]
    fn test_response_roundtrip() {
        let resp = RpcResponse::success(Some(Value::Number(1.into())), serde_json::json!({"status": "ok"}));
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: RpcResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.result.unwrap()["status"], "ok");
        assert!(parsed.error.is_none());
    }

    #[test]
    fn test_success_serializes_without_error() {
        let resp = RpcResponse::success(Some(Value::Number(1.into())), serde_json::json!("ok"));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_error_serializes_without_result() {
        let resp = RpcResponse::error(None, -32700, "Parse error");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("result"));
    }
}
