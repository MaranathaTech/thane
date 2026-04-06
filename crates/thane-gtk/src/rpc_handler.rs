use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{json, Value};
use thane_browser::scripting::{self, ACCESSIBILITY_TREE_JS};
use thane_browser::traits::BrowserSurface;
use gdk4::prelude::TextureExt;
use glib::object::Cast;
use vte4::GskRendererExt;
use gtk4::prelude::{NativeExt, SnapshotExt, WidgetExt as Gtk4WidgetExt};
use webkit6::prelude::WebViewExt;
use thane_core::audit::{AuditEventType, AuditSeverity};
use thane_core::notification::Notification;
use thane_core::pane::Orientation as PaneOrientation;
use thane_core::panel::PanelId;
use thane_rpc::methods::Method;
use thane_rpc::protocol::{RpcRequest, RpcResponse};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::window::AppStateHandle;

/// Capture a GTK widget as a PNG image using the GTK4 snapshot pipeline.
///
/// Uses `WidgetPaintable` to render the widget content to a texture, avoiding
/// trait method conflicts between gtk4 and vte4 preludes.
fn capture_widget_as_png(widget: &gtk4::Widget) -> Result<(glib::Bytes, i32, i32), String> {
    let width = widget.width();
    let height = widget.height();
    if width <= 0 || height <= 0 {
        return Err("Widget has zero size".to_string());
    }

    let paintable = gtk4::WidgetPaintable::new(Some(widget));
    let snapshot = gtk4::Snapshot::new();
    {
        use gdk4::prelude::PaintableExt;
        paintable.snapshot(snapshot.upcast_ref::<gdk4::Snapshot>(), width as f64, height as f64);
    }
    let node = snapshot
        .to_node()
        .ok_or_else(|| "Failed to create render node".to_string())?;

    let native = widget
        .native()
        .ok_or_else(|| "Widget has no native surface".to_string())?;
    let renderer = native
        .renderer()
        .ok_or_else(|| "No renderer available".to_string())?;

    let bounds = gtk4::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture = renderer.render_texture(&node, Some(&bounds));
    let png_bytes = texture.save_to_png_bytes();
    Ok((png_bytes, texture.width(), texture.height()))
}

/// A pending RPC request waiting for processing on the GTK main thread.
struct PendingRpc {
    request: RpcRequest,
    responder: oneshot::Sender<RpcResponse>,
}

/// Start the RPC bridge between the tokio IPC server and the GTK main thread.
///
/// Returns an async handler function that can be passed to `thane_ipc::server::start_server`.
/// The handler sends requests to the GTK main loop via a channel and awaits the response.
pub(crate) fn start_rpc_bridge(
    state: Rc<RefCell<AppStateHandle>>,
) -> thane_ipc::client::AsyncRpcHandler {
    let (tx, mut rx) = mpsc::unbounded_channel::<PendingRpc>();

    glib::idle_add_local(move || {
        while let Ok(pending) = rx.try_recv() {
            let mut responder = Some(pending.responder);
            let response = handle_request(&state, &pending.request, &mut responder);
            if let (Some(response), Some(responder)) = (response, responder) {
                let _ = responder.send(response);
            }
        }
        glib::ControlFlow::Continue
    });

    Arc::new(move |request: RpcRequest| {
        let tx = tx.clone();
        Box::pin(async move {
            let (resp_tx, resp_rx) = oneshot::channel();

            if tx
                .send(PendingRpc {
                    request: request.clone(),
                    responder: resp_tx,
                })
                .is_err()
            {
                return RpcResponse::internal_error(request.id, "GTK main loop not running");
            }

            match resp_rx.await {
                Ok(response) => response,
                Err(_) => RpcResponse::internal_error(request.id, "Handler dropped"),
            }
        })
    })
}

/// Dispatch an RPC request to the appropriate handler.
///
/// For most methods, returns `Some(response)`. For async methods (like screenshot),
/// takes ownership of the responder via `responder.take()` and returns `None` —
/// the response will be sent from an async callback instead.
fn handle_request(
    state: &Rc<RefCell<AppStateHandle>>,
    request: &RpcRequest,
    responder: &mut Option<oneshot::Sender<RpcResponse>>,
) -> Option<RpcResponse> {
    let method = match Method::parse(&request.method) {
        Some(m) => m,
        None => {
            return Some(RpcResponse::method_not_found(
                request.id.clone(),
                &request.method,
            ))
        }
    };

    let id = request.id.clone();
    let params = &request.params;

    match method {
        Method::Ping => Some(RpcResponse::success(id, json!("pong"))),
        Method::GetVersion => Some(RpcResponse::success(
            id,
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "git_hash": option_env!("GIT_HASH"),
            }),
        )),

        Method::WorkspaceList => {
            let s = state.borrow();
            let workspaces: Vec<Value> = s
                .workspace_mgr()
                .list()
                .iter()
                .map(|ws| {
                    json!({
                        "id": ws.id,
                        "title": ws.title,
                        "cwd": ws.cwd,
                        "tag": ws.tag,
                        "pane_count": ws.pane_count(),
                        "panel_count": ws.panels.len(),
                        "unread_notifications": ws.notifications.unread_count(),
                        "git_branch": ws.sidebar.git_branch,
                        "last_prompt": ws.sidebar.last_prompt,
                    })
                })
                .collect();
            Some(RpcResponse::success(
                id,
                json!({
                    "workspaces": workspaces,
                    "active_index": s.workspace_mgr().active_index(),
                }),
            ))
        }

        Method::WorkspaceCreate => {
            let title = params
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let cwd = params
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let mut s = state.borrow_mut();
            let title = title.unwrap_or_else(|| {
                let count = s.workspace_mgr().count() + 1;
                format!("Workspace {count}")
            });
            let cwd = cwd.unwrap_or_else(|| s.default_cwd());
            let (ws_id, _panel_id) = s.create_workspace(&title, &cwd);
            Some(RpcResponse::success(id, json!({ "id": ws_id })))
        }

        Method::WorkspaceSelect => {
            let mut s = state.borrow_mut();
            if let Some(index) = params.get("index").and_then(|v| v.as_u64()) {
                s.select_workspace(index as usize);
                Some(RpcResponse::success(id, json!({ "ok": true })))
            } else if let Some(id_str) = params.get("id").and_then(|v| v.as_str()) {
                if let Ok(ws_id) = Uuid::parse_str(id_str) {
                    if s.workspace_mgr_mut().select_by_id(ws_id) {
                        s.switch_to_active_workspace();
                        s.refresh_sidebar();
                        Some(RpcResponse::success(id, json!({ "ok": true })))
                    } else {
                        Some(RpcResponse::invalid_params(id, "Workspace not found"))
                    }
                } else {
                    Some(RpcResponse::invalid_params(id, "Invalid workspace ID"))
                }
            } else {
                Some(RpcResponse::invalid_params(id, "Provide index or id"))
            }
        }

        Method::WorkspaceClose => {
            let mut s = state.borrow_mut();
            s.close_active_workspace();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::WorkspaceRename => {
            let title = match params.get("title").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return Some(RpcResponse::invalid_params(id, "Missing title")),
            };
            let mut s = state.borrow_mut();
            s.workspace_mgr_mut().rename_active(title);
            s.refresh_sidebar();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::WorkspaceGetInfo => {
            let s = state.borrow();
            match s.workspace_mgr().active() {
                Some(ws) => Some(RpcResponse::success(
                    id,
                    json!({
                        "id": ws.id,
                        "title": ws.title,
                        "cwd": ws.cwd,
                        "tag": ws.tag,
                        "pane_count": ws.pane_count(),
                        "panel_count": ws.panels.len(),
                        "unread_notifications": ws.notifications.unread_count(),
                        "git_branch": ws.sidebar.git_branch,
                        "last_prompt": ws.sidebar.last_prompt,
                    }),
                )),
                None => Some(RpcResponse::error(id, -1, "No active workspace")),
            }
        }

        Method::SurfaceSplitRight => {
            let mut s = state.borrow_mut();
            s.split_pane(PaneOrientation::Horizontal);
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }
        Method::SurfaceSplitDown => {
            let mut s = state.borrow_mut();
            s.split_pane(PaneOrientation::Vertical);
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }
        Method::SurfaceClose => {
            let mut s = state.borrow_mut();
            s.close_focused_pane();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }
        Method::SurfaceFocusNext => {
            let mut s = state.borrow_mut();
            s.focus_next_pane();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }
        Method::SurfaceFocusPrev => {
            let mut s = state.borrow_mut();
            s.focus_prev_pane();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }
        Method::SurfaceFocusDirection => {
            let dir = params
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("right");
            let mut s = state.borrow_mut();
            match dir {
                "right" | "down" => s.focus_next_pane(),
                _ => s.focus_prev_pane(),
            }
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }
        Method::SurfaceZoomToggle => {
            let mut s = state.borrow_mut();
            s.toggle_pane_zoom();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::NotificationList => {
            let s = state.borrow();
            match s.workspace_mgr().active() {
                Some(ws) => {
                    let limit = params
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(20) as usize;
                    let notifications: Vec<Value> = ws
                        .notifications
                        .all()
                        .iter()
                        .rev()
                        .take(limit)
                        .map(|n| {
                            json!({
                                "id": n.id,
                                "title": n.title,
                                "body": n.body,
                                "read": n.read,
                                "timestamp": n.timestamp.to_rfc3339(),
                            })
                        })
                        .collect();
                    Some(RpcResponse::success(
                        id,
                        json!({ "notifications": notifications }),
                    ))
                }
                None => Some(RpcResponse::error(id, -1, "No active workspace")),
            }
        }

        Method::NotificationMarkRead => {
            let mut s = state.borrow_mut();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                ws.notifications.mark_all_read();
            }
            s.refresh_sidebar();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::NotificationClear => {
            let mut s = state.borrow_mut();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                ws.notifications.clear();
            }
            s.refresh_sidebar();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::NotificationSend => {
            let title = params
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Notification");
            let body = params
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut s = state.borrow_mut();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                let panel_id = ws
                    .focused_panel()
                    .map(|p| p.id)
                    .unwrap_or(Uuid::new_v4());
                ws.notifications
                    .push(Notification::new(panel_id, title, body));
            }
            s.refresh_sidebar();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::SidebarGetMetadata => {
            let s = state.borrow();
            match s.workspace_mgr().active() {
                Some(ws) => Some(RpcResponse::success(
                    id,
                    json!({
                        "git_branch": ws.sidebar.git_branch,
                        "git_dirty": ws.sidebar.git_dirty,
                        "ports": ws.sidebar.ports,
                        "last_prompt": ws.sidebar.last_prompt,
                    }),
                )),
                None => Some(RpcResponse::error(id, -1, "No active workspace")),
            }
        }
        Method::SidebarSetStatus => {
            let label = params
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let value = params
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let style_str = params
                .get("style")
                .and_then(|v| v.as_str())
                .unwrap_or("normal");
            let style = match style_str.to_lowercase().as_str() {
                "success" => thane_core::sidebar::StatusStyle::Success,
                "warning" => thane_core::sidebar::StatusStyle::Warning,
                "error" => thane_core::sidebar::StatusStyle::Error,
                "muted" => thane_core::sidebar::StatusStyle::Muted,
                _ => thane_core::sidebar::StatusStyle::Normal,
            };
            let mut s = state.borrow_mut();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                ws.sidebar
                    .status_entries
                    .push(thane_core::sidebar::StatusEntry {
                        label,
                        value,
                        style,
                    });
            }
            s.refresh_sidebar();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::BrowserOpen => {
            let url = params
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("about:blank");
            let mut s = state.borrow_mut();
            match s.open_browser(url) {
                Some((_ws_id, panel_id)) => {
                    Some(RpcResponse::success(id, json!({ "panel_id": panel_id })))
                }
                None => Some(RpcResponse::error(id, -1, "Failed to open browser")),
            }
        }

        Method::BrowserNavigate => {
            let url = match params.get("url").and_then(|v| v.as_str()) {
                Some(u) => u,
                None => return Some(RpcResponse::invalid_params(id, "Missing url")),
            };
            let panel_id = params
                .get("panel_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let s = state.borrow();
            if let Some(pid) = panel_id {
                if s.browser_navigate(pid, url) {
                    Some(RpcResponse::success(id, json!({ "ok": true })))
                } else {
                    Some(RpcResponse::error(id, -1, "Browser panel not found"))
                }
            } else if let Some(bp) = s.focused_browser_panel() {
                bp.surface().navigate(url);
                Some(RpcResponse::success(id, json!({ "ok": true })))
            } else {
                Some(RpcResponse::error(id, -1, "No active browser panel"))
            }
        }

        Method::BrowserEvalJs => {
            let script = match params.get("script").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => return Some(RpcResponse::invalid_params(id, "Missing script")),
            };
            let s = state.borrow();
            let panel_id: Option<PanelId> = params
                .get("panel_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let bp = if let Some(pid) = panel_id {
                s.browser_panel(&pid)
            } else {
                s.focused_browser_panel()
            };
            match bp {
                Some(bp) => {
                    bp.surface().eval_js(&script, Box::new(|_result| {}));
                    Some(RpcResponse::success(
                        id,
                        json!({ "ok": true, "note": "Script dispatched asynchronously" }),
                    ))
                }
                None => Some(RpcResponse::error(id, -1, "No active browser panel")),
            }
        }

        Method::BrowserGetAccessibilityTree => {
            let s = state.borrow();
            let panel_id: Option<PanelId> = params
                .get("panel_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let bp = if let Some(pid) = panel_id {
                s.browser_panel(&pid)
            } else {
                s.focused_browser_panel()
            };
            match bp {
                Some(bp) => {
                    bp.surface()
                        .eval_js(ACCESSIBILITY_TREE_JS, Box::new(|_result| {}));
                    Some(RpcResponse::success(
                        id,
                        json!({ "ok": true, "note": "Accessibility tree requested asynchronously" }),
                    ))
                }
                None => Some(RpcResponse::error(id, -1, "No active browser panel")),
            }
        }

        Method::BrowserClickElement => {
            let selector = match params.get("selector").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => return Some(RpcResponse::invalid_params(id, "Missing selector")),
            };
            let s = state.borrow();
            match s.focused_browser_panel() {
                Some(bp) => {
                    let js = scripting::click_element_js(selector);
                    bp.surface().eval_js(&js, Box::new(|_| {}));
                    Some(RpcResponse::success(id, json!({ "ok": true })))
                }
                None => Some(RpcResponse::error(id, -1, "No active browser panel")),
            }
        }

        Method::BrowserTypeText => {
            let selector = match params.get("selector").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => return Some(RpcResponse::invalid_params(id, "Missing selector")),
            };
            let text = match params.get("text").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return Some(RpcResponse::invalid_params(id, "Missing text")),
            };
            let s = state.borrow();
            match s.focused_browser_panel() {
                Some(bp) => {
                    let js = scripting::type_text_js(selector, text);
                    bp.surface().eval_js(&js, Box::new(|_| {}));
                    Some(RpcResponse::success(id, json!({ "ok": true })))
                }
                None => Some(RpcResponse::error(id, -1, "No active browser panel")),
            }
        }

        Method::BrowserScreenshot => {
            let s = state.borrow();
            let panel_id: Option<PanelId> = params
                .get("panel_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let bp = if let Some(pid) = panel_id {
                s.browser_panel(&pid)
            } else {
                s.focused_browser_panel()
            };
            let bp = match bp {
                Some(bp) => bp,
                None => return Some(RpcResponse::error(id, -1, "No active browser panel")),
            };

            let full_page = params
                .get("full_page")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let region_str = params
                .get("region")
                .and_then(|v| v.as_str())
                .unwrap_or("visible");
            let snapshot_region = if full_page || region_str == "full_document" {
                webkit6::SnapshotRegion::FullDocument
            } else {
                webkit6::SnapshotRegion::Visible
            };

            let web_view = bp.surface().web_view().clone();
            let resp_sender = responder.take().expect("responder must be available");

            web_view.snapshot(
                snapshot_region,
                webkit6::SnapshotOptions::NONE,
                None::<&gio::Cancellable>,
                move |result: Result<gdk4::Texture, glib::Error>| {
                    let response = match result {
                        Ok(texture) => {
                            let bytes = texture.save_to_png_bytes();
                            let b64: String = glib::base64_encode(bytes.as_ref()).into();
                            RpcResponse::success(
                                id,
                                json!({
                                    "image": b64,
                                    "format": "png",
                                    "width": texture.width(),
                                    "height": texture.height(),
                                }),
                            )
                        }
                        Err(e) => {
                            RpcResponse::error(id, -1, format!("Screenshot failed: {e}"))
                        }
                    };
                    let _ = resp_sender.send(response);
                },
            );
            None
        }

        Method::TerminalScreenshot => {
            let s = state.borrow();
            let panel_id: Option<PanelId> = params
                .get("panel_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let tp = if let Some(pid) = panel_id {
                s.terminal_panel(&pid)
            } else {
                s.focused_terminal_panel()
            };
            match tp {
                Some(tp) => {
                    let vte: &gtk4::Widget = tp.surface().vte_terminal().upcast_ref();
                    match capture_widget_as_png(vte) {
                        Ok((png_bytes, w, h)) => {
                            let b64: String =
                                glib::base64_encode(png_bytes.as_ref()).into();
                            Some(RpcResponse::success(
                                id,
                                json!({
                                    "image": b64,
                                    "format": "png",
                                    "width": w,
                                    "height": h,
                                }),
                            ))
                        }
                        Err(e) => Some(RpcResponse::error(
                            id,
                            -1,
                            format!("Terminal screenshot failed: {e}"),
                        )),
                    }
                }
                None => Some(RpcResponse::error(id, -1, "No active terminal panel")),
            }
        }

        Method::SandboxStatus => {
            let s = state.borrow();
            let ws_id = params
                .get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let ws = if let Some(wid) = ws_id {
                s.workspace_mgr().get(wid)
            } else {
                s.workspace_mgr().active()
            };
            match ws {
                Some(ws) => {
                    let policy = &ws.sandbox_policy;
                    Some(RpcResponse::success(
                        id,
                        json!({
                            "enabled": policy.enabled,
                            "root_dir": policy.root_dir,
                            "enforcement": format!("{:?}", policy.enforcement),
                            "read_only_paths": policy.read_only_paths,
                            "read_write_paths": policy.read_write_paths,
                            "denied_paths": policy.denied_paths,
                            "allow_network": policy.allow_network,
                            "landlock_supported": thane_platform::is_landlock_supported(),
                        }),
                    ))
                }
                None => Some(RpcResponse::error(id, -1, "No active workspace")),
            }
        }

        Method::SandboxEnable => {
            let mut s = state.borrow_mut();
            let mut ws_id = None;
            let mut ws_title = String::new();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                ws.sandbox_policy.enabled = true;
                ws.sandbox_policy.root_dir = std::path::PathBuf::from(&ws.cwd);
                ws.sandbox_policy.read_write_paths = vec![std::path::PathBuf::from(&ws.cwd)];
                ws_id = Some(ws.id);
                ws_title = ws.title.clone();
                tracing::info!("Sandbox enabled for workspace '{ws_title}'");
            }
            if let Some(wid) = ws_id {
                s.audit_log_mut().log(
                    wid,
                    None,
                    AuditEventType::SandboxToggle,
                    AuditSeverity::Warning,
                    format!("Sandbox enabled for workspace '{ws_title}' via RPC"),
                    json!({ "enabled": true }),
                );
            }
            s.refresh_sidebar();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::SandboxDisable => {
            let mut s = state.borrow_mut();
            let mut ws_id = None;
            let mut ws_title = String::new();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                ws.sandbox_policy.enabled = false;
                ws_id = Some(ws.id);
                ws_title = ws.title.clone();
                tracing::info!("Sandbox disabled for workspace '{ws_title}'");
            }
            if let Some(wid) = ws_id {
                s.audit_log_mut().log(
                    wid,
                    None,
                    AuditEventType::SandboxToggle,
                    AuditSeverity::Warning,
                    format!("Sandbox disabled for workspace '{ws_title}' via RPC"),
                    json!({ "enabled": false }),
                );
            }
            s.refresh_sidebar();
            Some(RpcResponse::success(id, json!({ "ok": true })))
        }

        Method::SandboxAllow => {
            let path_str = match params.get("path").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => return Some(RpcResponse::invalid_params(id, "Missing path")),
            };
            let path = std::path::PathBuf::from(&path_str);
            let read_only = params
                .get("read_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut s = state.borrow_mut();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                let ws_id = ws.id;
                let access = if read_only { "read_only" } else { "read_write" };
                if read_only {
                    if !ws.sandbox_policy.read_only_paths.contains(&path) {
                        ws.sandbox_policy.read_only_paths.push(path);
                    }
                } else if !ws.sandbox_policy.read_write_paths.contains(&path) {
                    ws.sandbox_policy.read_write_paths.push(path);
                }
                s.audit_log_mut().log(
                    ws_id,
                    None,
                    AuditEventType::SandboxPolicyChange,
                    AuditSeverity::Info,
                    format!("Sandbox: allowed {access} access to '{path_str}'"),
                    json!({ "path": path_str, "access": access }),
                );
                Some(RpcResponse::success(id, json!({ "ok": true })))
            } else {
                Some(RpcResponse::error(id, -1, "No active workspace"))
            }
        }

        Method::SandboxDeny => {
            let path_str = match params.get("path").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => return Some(RpcResponse::invalid_params(id, "Missing path")),
            };
            let path = std::path::PathBuf::from(&path_str);
            let mut s = state.borrow_mut();
            if let Some(ws) = s.workspace_mgr_mut().active_mut() {
                let ws_id = ws.id;
                if !ws.sandbox_policy.denied_paths.contains(&path) {
                    ws.sandbox_policy.denied_paths.push(path);
                }
                s.audit_log_mut().log(
                    ws_id,
                    None,
                    AuditEventType::SandboxPolicyChange,
                    AuditSeverity::Info,
                    format!("Sandbox: denied access to '{path_str}'"),
                    json!({ "path": path_str }),
                );
                Some(RpcResponse::success(id, json!({ "ok": true })))
            } else {
                Some(RpcResponse::error(id, -1, "No active workspace"))
            }
        }

        Method::AgentQueueSubmit => {
            let content = match params.get("content").and_then(|v| v.as_str()) {
                Some(c) => c.to_string(),
                None => return Some(RpcResponse::invalid_params(id, "Missing content")),
            };
            let workspace_id = params
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let priority = params
                .get("priority")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            let depends_on = params
                .get("depends_on")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let content_preview: String = content.chars().take(200).collect();
            let mut s = state.borrow_mut();
            let entry_id = s.agent_queue_mut().submit_with_depends(content, workspace_id, priority, depends_on);
            s.audit_log_mut().log(
                workspace_id.unwrap_or(Uuid::nil()),
                None,
                AuditEventType::QueueTaskSubmitted,
                AuditSeverity::Info,
                format!("Queue task submitted: {content_preview}"),
                json!({"entry_id": entry_id.to_string(), "priority": priority, "depends_on": depends_on.map(|id| id.to_string()), "source": "rpc"}),
            );
            Some(RpcResponse::success(id, json!({ "entry_id": entry_id })))
        }

        Method::AgentQueueList => {
            let s = state.borrow();
            let entries: Vec<Value> = s
                .agent_queue()
                .list()
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "status": p.status,
                        "priority": p.priority,
                        "content": p.content.chars().take(200).collect::<String>(),
                        "created_at": p.created_at.to_rfc3339(),
                        "started_at": p.started_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                        "completed_at": p.completed_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                        "error": p.error,
                        "depends_on": p.depends_on,
                        "tokens_used": {
                            "input_tokens": p.tokens_used.input_tokens,
                            "output_tokens": p.tokens_used.output_tokens,
                            "estimated_cost_usd": p.tokens_used.estimated_cost_usd,
                        },
                    })
                })
                .collect();
            Some(RpcResponse::success(
                id,
                json!({
                    "entries": entries,
                    "token_limit_paused": s.agent_queue().token_limit_paused,
                    "queued_count": s.agent_queue().queued_count(),
                    "running_count": s.agent_queue().running_count(),
                }),
            ))
        }

        Method::AgentQueueStatus => {
            let entry_id = match params
                .get("entry_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
            {
                Some(id) => id,
                None => {
                    return Some(RpcResponse::invalid_params(
                        id,
                        "Missing or invalid entry_id",
                    ))
                }
            };
            let s = state.borrow();
            match s.agent_queue().get(entry_id) {
                Some(p) => Some(RpcResponse::success(
                    id,
                    json!({
                        "id": p.id,
                        "status": p.status,
                        "priority": p.priority,
                        "content": p.content,
                        "created_at": p.created_at.to_rfc3339(),
                        "started_at": p.started_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                        "completed_at": p.completed_at.map(|t: chrono::DateTime<chrono::Utc>| t.to_rfc3339()),
                        "error": p.error,
                        "depends_on": p.depends_on,
                        "tokens_used": {
                            "input_tokens": p.tokens_used.input_tokens,
                            "output_tokens": p.tokens_used.output_tokens,
                            "estimated_cost_usd": p.tokens_used.estimated_cost_usd,
                        },
                    }),
                )),
                None => Some(RpcResponse::error(id, -1, "Entry not found")),
            }
        }

        Method::AgentQueueCancel => {
            let entry_id = match params
                .get("entry_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok())
            {
                Some(id) => id,
                None => {
                    return Some(RpcResponse::invalid_params(
                        id,
                        "Missing or invalid entry_id",
                    ))
                }
            };
            let mut s = state.borrow_mut();
            let ws_id = s.agent_queue().get(entry_id)
                .and_then(|e| e.workspace_id)
                .unwrap_or(Uuid::nil());
            if s.agent_queue_mut().cancel(entry_id) {
                s.audit_log_mut().log(
                    ws_id,
                    None,
                    AuditEventType::QueueTaskCancelled,
                    AuditSeverity::Info,
                    format!("Queue task cancelled: {entry_id}"),
                    json!({"entry_id": entry_id.to_string(), "source": "rpc"}),
                );
                Some(RpcResponse::success(id, json!({ "ok": true })))
            } else {
                Some(RpcResponse::error(id, -1, "Entry not found"))
            }
        }

        Method::AuditList => {
            let s = state.borrow();
            let min_severity = params
                .get("severity")
                .and_then(|v| v.as_str())
                .and_then(|s| match s.to_lowercase().as_str() {
                    "info" => Some(thane_core::audit::AuditSeverity::Info),
                    "warning" | "warn" => Some(thane_core::audit::AuditSeverity::Warning),
                    "alert" => Some(thane_core::audit::AuditSeverity::Alert),
                    "critical" | "crit" => Some(thane_core::audit::AuditSeverity::Critical),
                    _ => None,
                });
            let limit = params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;
            let events: Vec<&thane_core::audit::AuditEvent> = match min_severity {
                Some(sev) => s.audit_log().by_severity(sev),
                None => s.audit_log().all().iter().collect(),
            };
            let total = events.len();
            let truncated: Vec<Value> = events
                .iter()
                .rev()
                .take(limit)
                .map(|e| {
                    json!({
                        "id": e.id.to_string(),
                        "timestamp": e.timestamp.to_rfc3339(),
                        "workspace_id": e.workspace_id.to_string(),
                        "panel_id": e.panel_id.map(|id| id.to_string()),
                        "event_type": format!("{:?}", e.event_type),
                        "severity": format!("{:?}", e.severity),
                        "description": e.description,
                    })
                })
                .collect();
            Some(RpcResponse::success(
                id,
                json!({
                    "events": truncated,
                    "total": total,
                    "returned": truncated.len(),
                }),
            ))
        }

        Method::AuditExport => {
            let s = state.borrow();
            match s.audit_log().export_json() {
                Ok(json_str) => match serde_json::from_str::<Value>(&json_str) {
                    Ok(val) => Some(RpcResponse::success(id, json!({ "events": val }))),
                    Err(e) => Some(RpcResponse::error(
                        id,
                        -1,
                        format!("JSON parse error: {e}"),
                    )),
                },
                Err(e) => Some(RpcResponse::error(id, -1, format!("Export error: {e}"))),
            }
        }

        Method::AuditClear => {
            let mut s = state.borrow_mut();
            s.audit_log_mut().clear();
            Some(RpcResponse::success(
                id,
                json!({ "ok": true, "message": "Audit log cleared" }),
            ))
        }

        Method::AuditSetSensitivePolicy => {
            let action_str = match params.get("action").and_then(|v| v.as_str()) {
                Some(a) => a,
                None => {
                    return Some(RpcResponse::invalid_params(
                        id,
                        "Missing 'action' param (allow, warn, block)",
                    ))
                }
            };
            let action = match action_str.to_lowercase().as_str() {
                "allow" => thane_core::audit::SensitiveOpAction::Allow,
                "warn" => thane_core::audit::SensitiveOpAction::Warn,
                "block" => thane_core::audit::SensitiveOpAction::Block,
                _ => {
                    return Some(RpcResponse::invalid_params(
                        id,
                        "Invalid action: must be 'allow', 'warn', or 'block'",
                    ))
                }
            };
            let ws_id = params
                .get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let mut s = state.borrow_mut();
            let ws = if let Some(wid) = ws_id {
                s.workspace_mgr_mut().get_mut(wid)
            } else {
                s.workspace_mgr_mut().active_mut()
            };
            match ws {
                Some(ws) => {
                    ws.sensitive_op_action = action;
                    Some(RpcResponse::success(
                        id,
                        json!({ "ok": true, "action": format!("{action:?}") }),
                    ))
                }
                None => Some(RpcResponse::error(id, -1, "Workspace not found")),
            }
        }

        Method::WorkspaceHistoryList => {
            let s = state.borrow();
            let entries: Vec<Value> = s
                .workspace_history()
                .list()
                .iter()
                .map(|r| {
                    json!({
                        "original_id": r.original_id,
                        "title": r.title,
                        "cwd": r.cwd,
                        "tag": r.tag,
                        "closed_at": r.closed_at.to_rfc3339(),
                    })
                })
                .collect();
            Some(RpcResponse::success(
                id,
                json!({ "entries": entries, "count": entries.len() }),
            ))
        }

        Method::WorkspaceHistoryReopen => {
            let id_str = match params.get("id").and_then(|v| v.as_str()) {
                Some(s) => s,
                None => {
                    return Some(RpcResponse::invalid_params(
                        id,
                        "Missing 'id' (original workspace UUID)",
                    ))
                }
            };
            let original_id = match Uuid::parse_str(id_str) {
                Ok(u) => u,
                Err(_) => return Some(RpcResponse::invalid_params(id, "Invalid UUID")),
            };
            let mut s = state.borrow_mut();
            match s.reopen_from_history(original_id) {
                Some((ws_id, _panel_id)) => {
                    Some(RpcResponse::success(id, json!({ "id": ws_id })))
                }
                None => Some(RpcResponse::error(id, -1, "Entry not found in history")),
            }
        }

        Method::WorkspaceHistoryClear => {
            let mut s = state.borrow_mut();
            s.workspace_history_mut().clear();
            s.save_history();
            s.refresh_sidebar();
            Some(RpcResponse::success(
                id,
                json!({ "ok": true, "message": "History cleared" }),
            ))
        }

        Method::GetConfig => {
            let s = state.borrow();
            let values: serde_json::Value =
                serde_json::to_value(s.config().all()).unwrap_or_default();
            Some(RpcResponse::success(id, values))
        }
    }
}
