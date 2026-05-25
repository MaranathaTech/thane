//! IPC handler for the daemon: services `ping`, `system.status`, and the
//! `agent_queue.*` namespace.

use std::sync::Arc;

use serde_json::{Value, json};
use thane_ipc::client::{AsyncRpcHandler, RpcFuture};
use thane_rpc::methods::Method;
use thane_rpc::protocol::{RpcRequest, RpcResponse};

use crate::DaemonState;

/// Build the JSON-RPC handler for the daemon's IPC server.
pub fn build_handler(state: Arc<DaemonState>) -> AsyncRpcHandler {
    Arc::new(move |req: RpcRequest| -> RpcFuture {
        let state = state.clone();
        Box::pin(async move { dispatch(state, req) })
    })
}

fn dispatch(state: Arc<DaemonState>, request: RpcRequest) -> RpcResponse {
    let id = request.id.clone();
    let Some(method) = Method::parse(&request.method) else {
        return RpcResponse::error(
            id,
            -32601,
            format!("Method not found: {}", request.method),
        );
    };

    let params = &request.params;
    match method {
        Method::Ping => RpcResponse::success(id, json!({"pong": true, "mode": "daemon"})),

        Method::GetVersion => RpcResponse::success(
            id,
            json!({"version": env!("CARGO_PKG_VERSION")}),
        ),

        Method::SystemStatus => RpcResponse::success(
            id,
            json!({
                "daemon_running": true,
                "daemon_pid": std::process::id(),
                "socket_path": state.socket_path().to_string_lossy(),
                "uptime_secs": state.uptime_secs(),
                "version": env!("CARGO_PKG_VERSION"),
            }),
        ),

        Method::AgentQueueSubmit => {
            let content = params["content"].as_str().unwrap_or("").to_string();
            if content.is_empty() {
                return RpcResponse::invalid_params(id, "Missing 'content' parameter");
            }
            let workspace_id = params["workspace_id"]
                .as_str()
                .and_then(|s| s.parse::<uuid::Uuid>().ok());
            let priority = params["priority"].as_i64().unwrap_or(0) as i32;

            let entry_id = state.with_queue(|q| q.submit(content, workspace_id, priority));
            RpcResponse::success(id, json!({"entry_id": entry_id}))
        }

        Method::AgentQueueList => {
            let (entries, token_limit_paused, queued_count, running_count) =
                state.with_queue(|q| {
                    let entries: Vec<Value> = q
                        .list()
                        .iter()
                        .map(|p| {
                            json!({
                                "id": p.id,
                                "status": p.status,
                                "priority": p.priority,
                                "created_at": p.created_at.to_rfc3339(),
                                "content_preview": &p.content[..p.content.len().min(100)],
                            })
                        })
                        .collect();
                    (entries, q.token_limit_paused, q.queued_count(), q.running_count())
                });
            RpcResponse::success(
                id,
                json!({
                    "entries": entries,
                    "token_limit_paused": token_limit_paused,
                    "queued_count": queued_count,
                    "running_count": running_count,
                }),
            )
        }

        Method::AgentQueueStatus => {
            let entry_id = match params["entry_id"].as_str().and_then(|s| s.parse().ok()) {
                Some(id) => id,
                None => return RpcResponse::invalid_params(id, "Missing or invalid entry_id"),
            };
            let response = state.with_queue(|q| {
                q.get(entry_id).map(|p| {
                    json!({
                        "id": p.id,
                        "status": p.status,
                        "content": p.content,
                        "priority": p.priority,
                        "created_at": p.created_at.to_rfc3339(),
                        "started_at": p.started_at.map(|t| t.to_rfc3339()),
                        "completed_at": p.completed_at.map(|t| t.to_rfc3339()),
                        "error": p.error,
                        "tokens_used": {
                            "input_tokens": p.tokens_used.input_tokens,
                            "output_tokens": p.tokens_used.output_tokens,
                            "estimated_cost_usd": p.tokens_used.estimated_cost_usd,
                        },
                    })
                })
            });
            match response {
                Some(v) => RpcResponse::success(id, v),
                None => RpcResponse::error(id, -1, "Entry not found"),
            }
        }

        Method::AgentQueueCancel => {
            let entry_id = match params["entry_id"].as_str().and_then(|s| s.parse().ok()) {
                Some(id) => id,
                None => return RpcResponse::invalid_params(id, "Missing or invalid entry_id"),
            };
            let ok = state.with_queue(|q| q.cancel(entry_id));
            if ok {
                RpcResponse::success(id, json!({"ok": true}))
            } else {
                RpcResponse::error(id, -1, "Entry not found")
            }
        }

        Method::AuditSinkStatus => match state.sink_status() {
            Some(report) => match serde_json::to_value(&report) {
                Ok(v) => RpcResponse::success(id, v),
                Err(e) => RpcResponse::internal_error(id, format!("serialize sinks: {e}")),
            },
            None => RpcResponse::success(id, json!({"sinks": []})),
        },

        Method::AuditDlqList => {
            let filter_sink = params["sink"].as_str();
            let limit = params["limit"].as_u64().unwrap_or(20) as usize;
            let dlq = state.dlq();
            let entries_res = match filter_sink {
                Some(name) => dlq.read_by_sink(name),
                None => dlq.read_all(),
            };
            match entries_res {
                Ok(mut entries) => {
                    // Newest first — file order is oldest-first.
                    entries.reverse();
                    let total = entries.len();
                    entries.truncate(limit);
                    let json_entries = serde_json::to_value(&entries)
                        .unwrap_or(Value::Array(vec![]));
                    RpcResponse::success(
                        id,
                        json!({
                            "total": total,
                            "returned": json_entries.as_array().map(|a| a.len()).unwrap_or(0),
                            "entries": json_entries,
                        }),
                    )
                }
                Err(e) => RpcResponse::internal_error(id, format!("DLQ read: {e}")),
            }
        }

        Method::AuditDlqRetry => {
            let filter_sink = params["sink"].as_str();
            let event_id = params["event_id"].as_str();
            let dispatcher = match state.dispatcher() {
                Some(d) => d.clone(),
                None => {
                    return RpcResponse::error(
                        id,
                        -1,
                        "No external sinks configured; nothing to retry",
                    );
                }
            };
            let dlq = state.dlq();
            let entries = match dlq.read_all() {
                Ok(v) => v,
                Err(e) => return RpcResponse::internal_error(id, format!("DLQ read: {e}")),
            };
            let mut count = 0u32;
            for entry in &entries {
                if let Some(s) = filter_sink
                    && entry.sink != s { continue; }
                if let Some(eid) = event_id
                    && entry.event.id.to_string() != eid { continue; }
                dispatcher.retry_event(&entry.event);
                count += 1;
            }
            RpcResponse::success(id, json!({"retried": count}))
        }

        Method::AuditDlqClear => {
            // Gate behind the same audit-allow-clear policy as the regular
            // audit log clear: never let an attacker wipe forensic evidence
            // by default.
            let config = state.config_snapshot();
            if !config.audit_allow_clear() {
                return RpcResponse::error(
                    id,
                    -32000,
                    "DLQ clear refused: set audit-allow-clear = true in config",
                );
            }
            let dlq = state.dlq();
            match dlq.clear() {
                Ok(()) => RpcResponse::success(id, json!({"ok": true})),
                Err(e) => RpcResponse::internal_error(id, format!("DLQ clear: {e}")),
            }
        }

        other => RpcResponse::error(
            id,
            -32601,
            format!("Method {other:?} is not available in daemon mode"),
        ),
    }
}
