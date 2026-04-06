//! thane-bridge: UniFFI bridge exposing the Rust core to Swift/Kotlin.
//!
//! This crate wraps `thane-core`, `thane-persist`, `thane-platform`, and `thane-ipc`
//! behind a UniFFI-compatible interface so that the macOS (and future mobile) frontends
//! can call into the shared Rust business logic.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use thane_core::agent_queue::{AgentQueue, QueueEntryStatus as CoreQueueStatus};
use thane_core::audit::{AuditEventType, AuditLog, AuditSeverity as CoreAuditSeverity};
use thane_core::config::Config;
use thane_core::notification::Urgency as CoreUrgency;
use thane_core::pane::Orientation;
use thane_core::panel::PanelType as CorePanelType;
use thane_core::sandbox::{EnforcementLevel as CoreEnforcement, SandboxPolicy};
use thane_core::session::{AppSnapshot, ClosedWorkspaceRecord, WorkspaceHistory};
use thane_core::workspace::WorkspaceManager;
use thane_persist::history_store::HistoryStore;
use thane_persist::store::SessionStore;
use thane_platform::traits::PlatformDirs;

uniffi::include_scaffolding!("thane_bridge");

// ─── Bridge error ──────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("Workspace not found")]
    WorkspaceNotFound,
    #[error("Panel not found")]
    PanelNotFound,
    #[error("Pane not found")]
    PaneNotFound,
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
    #[error("Config error: {0}")]
    ConfigError(String),
    #[error("Persistence error: {0}")]
    PersistenceError(String),
    #[error("IPC error: {0}")]
    IpcError(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<thane_core::error::CoreError> for BridgeError {
    fn from(e: thane_core::error::CoreError) -> Self {
        match e {
            thane_core::error::CoreError::WorkspaceNotFound(_) => BridgeError::WorkspaceNotFound,
            thane_core::error::CoreError::PanelNotFound(_) => BridgeError::PanelNotFound,
            thane_core::error::CoreError::PaneNotFound(_) => BridgeError::PaneNotFound,
            other => BridgeError::Internal(other.to_string()),
        }
    }
}

// ─── FFI-safe enum wrappers ────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitOrientation {
    Horizontal,
    Vertical,
}

impl From<SplitOrientation> for Orientation {
    fn from(o: SplitOrientation) -> Self {
        match o {
            SplitOrientation::Horizontal => Orientation::Horizontal,
            SplitOrientation::Vertical => Orientation::Vertical,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePanelType {
    Terminal,
    Browser,
}

impl From<CorePanelType> for BridgePanelType {
    fn from(pt: CorePanelType) -> Self {
        match pt {
            CorePanelType::Terminal => BridgePanelType::Terminal,
            CorePanelType::Browser => BridgePanelType::Browser,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeEnforcementLevel {
    Permissive,
    Enforcing,
    Strict,
}

impl From<CoreEnforcement> for BridgeEnforcementLevel {
    fn from(e: CoreEnforcement) -> Self {
        match e {
            CoreEnforcement::Permissive => BridgeEnforcementLevel::Permissive,
            CoreEnforcement::Enforcing => BridgeEnforcementLevel::Enforcing,
            CoreEnforcement::Strict => BridgeEnforcementLevel::Strict,
        }
    }
}

impl From<BridgeEnforcementLevel> for CoreEnforcement {
    fn from(e: BridgeEnforcementLevel) -> Self {
        match e {
            BridgeEnforcementLevel::Permissive => CoreEnforcement::Permissive,
            BridgeEnforcementLevel::Enforcing => CoreEnforcement::Enforcing,
            BridgeEnforcementLevel::Strict => CoreEnforcement::Strict,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAuditSeverity {
    Info,
    Warning,
    Alert,
    Critical,
}

impl From<CoreAuditSeverity> for BridgeAuditSeverity {
    fn from(s: CoreAuditSeverity) -> Self {
        match s {
            CoreAuditSeverity::Info => BridgeAuditSeverity::Info,
            CoreAuditSeverity::Warning => BridgeAuditSeverity::Warning,
            CoreAuditSeverity::Alert => BridgeAuditSeverity::Alert,
            CoreAuditSeverity::Critical => BridgeAuditSeverity::Critical,
        }
    }
}

impl From<BridgeAuditSeverity> for CoreAuditSeverity {
    fn from(s: BridgeAuditSeverity) -> Self {
        match s {
            BridgeAuditSeverity::Info => CoreAuditSeverity::Info,
            BridgeAuditSeverity::Warning => CoreAuditSeverity::Warning,
            BridgeAuditSeverity::Alert => CoreAuditSeverity::Alert,
            BridgeAuditSeverity::Critical => CoreAuditSeverity::Critical,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeQueueEntryStatus {
    Queued,
    Running,
    PausedTokenLimit,
    PausedByUser,
    Completed,
    Failed,
    Cancelled,
}

impl From<CoreQueueStatus> for BridgeQueueEntryStatus {
    fn from(s: CoreQueueStatus) -> Self {
        match s {
            CoreQueueStatus::Queued => BridgeQueueEntryStatus::Queued,
            CoreQueueStatus::Running => BridgeQueueEntryStatus::Running,
            CoreQueueStatus::PausedTokenLimit => BridgeQueueEntryStatus::PausedTokenLimit,
            CoreQueueStatus::PausedByUser => BridgeQueueEntryStatus::PausedByUser,
            CoreQueueStatus::Completed => BridgeQueueEntryStatus::Completed,
            CoreQueueStatus::Failed => BridgeQueueEntryStatus::Failed,
            CoreQueueStatus::Cancelled => BridgeQueueEntryStatus::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeNotifyUrgency {
    Low,
    Normal,
    Critical,
}

impl From<CoreUrgency> for BridgeNotifyUrgency {
    fn from(u: CoreUrgency) -> Self {
        match u {
            CoreUrgency::Low => BridgeNotifyUrgency::Low,
            CoreUrgency::Normal => BridgeNotifyUrgency::Normal,
            CoreUrgency::Critical => BridgeNotifyUrgency::Critical,
        }
    }
}

// ─── FFI-safe data transfer records ────────────────────────

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub tag: Option<String>,
    pub pane_count: u64,
    pub panel_count: u64,
    pub unread_notifications: u64,
}

#[derive(Debug, Clone)]
pub struct PanelInfo {
    pub id: String,
    pub panel_type: BridgePanelType,
    pub title: String,
    pub location: String,
    pub has_unread: bool,
}

#[derive(Debug, Clone)]
pub struct SplitResult {
    pub pane_id: String,
    pub panel_id: String,
}

#[derive(Debug, Clone)]
pub struct NotificationInfo {
    pub id: String,
    pub panel_id: String,
    pub title: String,
    pub body: String,
    pub urgency: BridgeNotifyUrgency,
    pub timestamp: String,
    pub read: bool,
}

#[derive(Debug, Clone)]
pub struct AuditEventInfo {
    pub id: String,
    pub timestamp: String,
    pub workspace_id: String,
    pub panel_id: Option<String>,
    pub event_type: String,
    pub severity: BridgeAuditSeverity,
    pub description: String,
    pub metadata_json: String,
}

#[derive(Debug, Clone)]
pub struct QueueEntryInfo {
    pub id: String,
    pub content: String,
    pub workspace_id: Option<String>,
    pub priority: i32,
    pub status: BridgeQueueEntryStatus,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub error: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    /// The model Claude Code selected for this task (e.g. "claude-sonnet-4-5-20250514").
    pub model: Option<String>,
    /// Task ID this entry depends on (must complete before this entry runs).
    pub depends_on: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct SandboxInfo {
    pub enabled: bool,
    pub root_dir: String,
    pub read_only_paths: Vec<String>,
    pub read_write_paths: Vec<String>,
    pub denied_paths: Vec<String>,
    pub allow_network: bool,
    pub max_open_files: Option<u64>,
    pub max_write_bytes: Option<u64>,
    pub max_cpu_seconds: Option<u64>,
    pub enforcement: BridgeEnforcementLevel,
}

#[derive(Debug, Clone)]
pub struct SandboxCommand {
    pub executable: String,
    pub args: Vec<String>,
    pub extra_env: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub restored: bool,
    pub workspace_count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TokenLimitsDTO {
    pub plan_name: String,
    pub has_caps: bool,
    /// "utilization" or "dollar" — controls what the UI shows as the primary metric.
    pub display_mode: String,
    pub five_hour_utilization: Option<f64>,
    pub five_hour_resets_at: Option<String>,
    pub seven_day_utilization: Option<f64>,
    pub seven_day_resets_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectCostDTO {
    pub session_cost_usd: f64,
    pub session_input_tokens: u64,
    pub session_output_tokens: u64,
    pub session_cache_read_tokens: u64,
    pub session_cache_write_tokens: u64,
    pub alltime_cost_usd: f64,
    pub alltime_input_tokens: u64,
    pub alltime_output_tokens: u64,
    pub alltime_cache_read_tokens: u64,
    pub alltime_cache_write_tokens: u64,
    pub session_count: u64,
    /// Plan name for display context.
    pub plan_name: String,
    /// "utilization" or "dollar".
    pub display_mode: String,
    /// 5-hour utilization percentage (0-100), if available.
    pub five_hour_utilization: Option<f64>,
    /// 7-day utilization percentage (0-100), if available.
    pub seven_day_utilization: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ClosedWorkspaceInfo {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub tag: Option<String>,
    pub closed_at: String,
}

// ─── Callback interface ────────────────────────────────────

pub trait UiCallback: Send + Sync {
    fn workspace_changed(&self, active_id: String);
    fn workspace_list_changed(&self);
    fn notification_received(&self, workspace_id: String, title: String, body: String);
    fn agent_status_changed(&self, workspace_id: String, active: bool);
    fn queue_entry_completed(&self, entry_id: String, success: bool);
    fn pane_layout_changed(&self, workspace_id: String);
    fn config_changed(&self);
}

// ─── Internal state ────────────────────────────────────────

struct BridgeState {
    workspace_mgr: WorkspaceManager,
    config: Config,
    audit_log: AuditLog,
    agent_queue: AgentQueue,
    session_store: SessionStore,
    #[allow(dead_code)]
    sessions_dir: PathBuf,
    workspace_history: WorkspaceHistory,
    history_store: HistoryStore,
    ui_callback: Option<Box<dyn UiCallback>>,
}

// ─── Main bridge object ────────────────────────────────────

pub struct ThaneBridge {
    state: Arc<Mutex<BridgeState>>,
}

impl ThaneBridge {
    pub fn new(config_path: Option<String>) -> Result<Self, BridgeError> {
        let config = if let Some(path) = config_path {
            Config::load(std::path::Path::new(&path))
                .map_err(|e| BridgeError::ConfigError(e.to_string()))?
        } else {
            Config::load_default()
        };

        // Use platform-appropriate directories.
        #[cfg(target_os = "macos")]
        let dirs = thane_platform::MacosDirs;
        #[cfg(target_os = "linux")]
        let dirs = thane_platform::LinuxDirs;

        let sessions_dir = dirs.sessions_dir();
        let session_store = SessionStore::new(sessions_dir.clone());
        let history_store = HistoryStore::new(sessions_dir.clone());
        let workspace_history = history_store.load().unwrap_or_default();

        if let Err(e) = dirs.ensure_dirs() {
            tracing::warn!("Failed to create platform directories: {e}");
        }

        let state = BridgeState {
            workspace_mgr: WorkspaceManager::new(),
            config,
            audit_log: AuditLog::new(10000),
            agent_queue: AgentQueue::new(),
            session_store,
            sessions_dir,
            workspace_history,
            history_store,
            ui_callback: None,
        };

        Ok(Self {
            state: Arc::new(Mutex::new(state)),
        })
    }

    // ── Workspace management ──

    pub fn create_workspace(&self, title: String, cwd: String) -> Result<WorkspaceInfo, BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.create(&title, &cwd);
        let info = workspace_to_info(ws);
        let callback = s.ui_callback.as_ref().map(|_| {
            info.id.clone()
        });
        drop(s);
        if let Some(id) = callback {
            if let Ok(s) = self.state.lock() {
                if let Some(cb) = &s.ui_callback {
                    cb.workspace_list_changed();
                    cb.workspace_changed(id);
                }
            }
        }
        Ok(info)
    }

    pub fn list_workspaces(&self) -> Vec<WorkspaceInfo> {
        let s = self.state.lock().unwrap();
        s.workspace_mgr.list().iter().map(workspace_to_info).collect()
    }

    pub fn select_workspace(&self, id: String) -> Result<bool, BridgeError> {
        let uuid = parse_uuid(&id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let result = s.workspace_mgr.select_by_id(uuid);
        if result {
            if let Some(cb) = &s.ui_callback {
                cb.workspace_changed(id);
            }
        }
        Ok(result)
    }

    pub fn close_workspace(&self, id: String) -> Result<bool, BridgeError> {
        let uuid = parse_uuid(&id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let closed = s.workspace_mgr.close_by_id(uuid).is_some();
        if closed {
            if let Some(cb) = &s.ui_callback {
                cb.workspace_list_changed();
                if let Some(active) = s.workspace_mgr.active() {
                    cb.workspace_changed(active.id.to_string());
                }
            }
        }
        Ok(closed)
    }

    pub fn rename_workspace(&self, id: String, title: String) -> Result<bool, BridgeError> {
        let uuid = parse_uuid(&id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        if let Some(ws) = s.workspace_mgr.get_mut(uuid) {
            ws.title = title;
            if let Some(cb) = &s.ui_callback {
                cb.workspace_list_changed();
            }
            Ok(true)
        } else {
            Err(BridgeError::WorkspaceNotFound)
        }
    }

    pub fn active_workspace(&self) -> Option<WorkspaceInfo> {
        let s = self.state.lock().ok()?;
        s.workspace_mgr.active().map(workspace_to_info)
    }

    // ── Pane / split operations ──

    pub fn split_terminal(&self, orientation: SplitOrientation) -> Result<SplitResult, BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.active_mut()
            .ok_or(BridgeError::WorkspaceNotFound)?;
        let (pane_id, panel_id) = ws.split_terminal(orientation.into())?;
        let ws_id = ws.id.to_string();
        if let Some(cb) = &s.ui_callback {
            cb.pane_layout_changed(ws_id);
        }
        Ok(SplitResult {
            pane_id: pane_id.to_string(),
            panel_id: panel_id.to_string(),
        })
    }

    pub fn split_browser(&self, url: String, orientation: SplitOrientation) -> Result<SplitResult, BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.active_mut()
            .ok_or(BridgeError::WorkspaceNotFound)?;
        let (pane_id, panel_id) = ws.split_browser(&url, orientation.into())?;
        let ws_id = ws.id.to_string();
        if let Some(cb) = &s.ui_callback {
            cb.pane_layout_changed(ws_id);
        }
        Ok(SplitResult {
            pane_id: pane_id.to_string(),
            panel_id: panel_id.to_string(),
        })
    }

    pub fn close_pane(&self) -> Result<(), BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.active_mut()
            .ok_or(BridgeError::WorkspaceNotFound)?;
        let focused = ws.focused_pane_id;
        ws.close_pane(focused)?;
        let ws_id = ws.id.to_string();
        if let Some(cb) = &s.ui_callback {
            cb.pane_layout_changed(ws_id);
        }
        Ok(())
    }

    pub fn focus_next_pane(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(ws) = s.workspace_mgr.active_mut() {
                ws.focus_next_pane();
            }
        }
    }

    pub fn focus_prev_pane(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(ws) = s.workspace_mgr.active_mut() {
                ws.focus_prev_pane();
            }
        }
    }

    pub fn focus_direction(&self, direction: String) -> Result<(), BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.active_mut()
            .ok_or(BridgeError::WorkspaceNotFound)?;
        match direction.as_str() {
            "next" => ws.focus_next_pane(),
            "prev" => ws.focus_prev_pane(),
            _ => return Err(BridgeError::InvalidOperation(
                format!("Unknown direction: {direction}. Use 'next' or 'prev'"),
            )),
        }
        Ok(())
    }

    // ── Panel management ──

    pub fn add_browser_panel(&self, url: String) -> Result<String, BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.active_mut()
            .ok_or(BridgeError::WorkspaceNotFound)?;
        let panel_id = ws.add_browser_to_focused_pane(&url)?;
        Ok(panel_id.to_string())
    }

    pub fn close_panel(&self, panel_id: String) -> Result<bool, BridgeError> {
        let pid = parse_panel_id(&panel_id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.active_mut()
            .ok_or(BridgeError::WorkspaceNotFound)?;
        if let Some(pane_id) = ws.pane_for_panel(pid) {
            let removed = ws.close_panel(pane_id, pid)?;
            let ws_id = ws.id.to_string();
            if let Some(cb) = &s.ui_callback {
                cb.pane_layout_changed(ws_id);
            }
            Ok(removed)
        } else {
            Err(BridgeError::PanelNotFound)
        }
    }

    pub fn select_panel(&self, panel_id: String) -> bool {
        if let Ok(pid) = parse_panel_id(&panel_id) {
            if let Ok(mut s) = self.state.lock() {
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    return ws.select_panel(pid);
                }
            }
        }
        false
    }

    pub fn reorder_panel(&self, panel_id: String, new_index: u32) -> bool {
        if let Ok(pid) = parse_panel_id(&panel_id) {
            if let Ok(mut s) = self.state.lock() {
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    return ws.reorder_panel(pid, new_index as usize);
                }
            }
        }
        false
    }

    pub fn next_panel(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(ws) = s.workspace_mgr.active_mut() {
                ws.next_panel();
            }
        }
    }

    pub fn prev_panel(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(ws) = s.workspace_mgr.active_mut() {
                ws.prev_panel();
            }
        }
    }

    pub fn list_panels(&self) -> Vec<PanelInfo> {
        let s = self.state.lock().unwrap();
        if let Some(ws) = s.workspace_mgr.active() {
            ws.panels.values().map(|p| PanelInfo {
                id: p.id.to_string(),
                panel_type: p.panel_type.into(),
                title: p.title.clone(),
                location: p.location.clone(),
                has_unread: p.has_unread,
            }).collect()
        } else {
            Vec::new()
        }
    }

    pub fn focused_panel(&self) -> Option<PanelInfo> {
        let s = self.state.lock().ok()?;
        let ws = s.workspace_mgr.active()?;
        ws.focused_panel().map(|p| PanelInfo {
            id: p.id.to_string(),
            panel_type: p.panel_type.into(),
            title: p.title.clone(),
            location: p.location.clone(),
            has_unread: p.has_unread,
        })
    }

    // ── Notifications ──

    pub fn list_notifications(&self, workspace_id: Option<String>) -> Vec<NotificationInfo> {
        let s = self.state.lock().unwrap();
        if let Some(ws_id) = workspace_id {
            if let Ok(uuid) = Uuid::parse_str(&ws_id) {
                if let Some(ws) = s.workspace_mgr.get(uuid) {
                    return ws.notifications.all().iter().map(notification_to_info).collect();
                }
            }
            Vec::new()
        } else if let Some(ws) = s.workspace_mgr.active() {
            ws.notifications.all().iter().map(notification_to_info).collect()
        } else {
            Vec::new()
        }
    }

    pub fn mark_notification_read(&self, notification_id: String) {
        if let Ok(uuid) = Uuid::parse_str(&notification_id) {
            if let Ok(mut s) = self.state.lock() {
                if let Some(ws) = s.workspace_mgr.active_mut() {
                    ws.notifications.mark_read(uuid);
                }
            }
        }
    }

    pub fn mark_all_notifications_read(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(ws) = s.workspace_mgr.active_mut() {
                ws.notifications.mark_all_read();
            }
        }
    }

    pub fn clear_notifications(&self) {
        if let Ok(mut s) = self.state.lock() {
            if let Some(ws) = s.workspace_mgr.active_mut() {
                ws.notifications.clear();
            }
        }
    }

    pub fn unread_notification_count(&self) -> u64 {
        if let Ok(s) = self.state.lock() {
            if let Some(ws) = s.workspace_mgr.active() {
                return ws.notifications.unread_count() as u64;
            }
        }
        0
    }

    // ── Audit log ──

    pub fn list_audit_events(&self, min_severity: Option<BridgeAuditSeverity>) -> Vec<AuditEventInfo> {
        let s = self.state.lock().unwrap();
        let events = if let Some(sev) = min_severity {
            s.audit_log.by_severity(sev.into())
        } else {
            s.audit_log.all().iter().collect()
        };
        events.iter().map(|e| AuditEventInfo {
            id: e.id.to_string(),
            timestamp: e.timestamp.to_rfc3339(),
            workspace_id: e.workspace_id.to_string(),
            panel_id: e.panel_id.map(|p| p.to_string()),
            event_type: format!("{:?}", e.event_type),
            severity: e.severity.into(),
            description: e.description.clone(),
            metadata_json: e.metadata.to_string(),
        }).collect()
    }

    pub fn export_audit_json(&self) -> String {
        let s = self.state.lock().unwrap();
        s.audit_log.export_json().unwrap_or_else(|_| "[]".to_string())
    }

    pub fn clear_audit_log(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.audit_log.clear();
        }
    }

    // ── Agent queue ──

    pub fn queue_submit(&self, content: String, workspace_id: Option<String>, priority: i32) -> String {
        self.queue_submit_with_depends(content, workspace_id, priority, None)
    }

    pub fn queue_submit_with_depends(&self, content: String, workspace_id: Option<String>, priority: i32, depends_on: Option<String>) -> String {
        let ws_uuid = workspace_id.and_then(|id| Uuid::parse_str(&id).ok());
        let dep_uuid = depends_on.and_then(|id| Uuid::parse_str(&id).ok());
        let content_preview: String = content.chars().take(200).collect();
        let mut s = self.state.lock().unwrap();
        let entry_id = s.agent_queue.submit_with_depends(content, ws_uuid, priority, dep_uuid);
        s.audit_log.log(
            ws_uuid.unwrap_or(Uuid::nil()),
            None,
            AuditEventType::QueueTaskSubmitted,
            CoreAuditSeverity::Info,
            format!("Queue task submitted: {content_preview}"),
            serde_json::json!({"entry_id": entry_id.to_string(), "priority": priority, "depends_on": dep_uuid.map(|id| id.to_string()), "source": "bridge"}),
        );
        entry_id.to_string()
    }

    pub fn queue_list(&self) -> Vec<QueueEntryInfo> {
        let s = self.state.lock().unwrap();
        s.agent_queue.list().iter().map(queue_entry_to_info).collect()
    }

    pub fn queue_status(&self, entry_id: String) -> Option<QueueEntryInfo> {
        let uuid = Uuid::parse_str(&entry_id).ok()?;
        let s = self.state.lock().ok()?;
        s.agent_queue.get(uuid).map(queue_entry_to_info)
    }

    pub fn queue_cancel(&self, entry_id: String) -> bool {
        if let Ok(uuid) = Uuid::parse_str(&entry_id) {
            if let Ok(mut s) = self.state.lock() {
                let ws_id = s.agent_queue.get(uuid)
                    .and_then(|e| e.workspace_id)
                    .unwrap_or(Uuid::nil());
                let cancelled = s.agent_queue.cancel(uuid);
                if cancelled {
                    s.audit_log.log(
                        ws_id,
                        None,
                        AuditEventType::QueueTaskCancelled,
                        CoreAuditSeverity::Info,
                        format!("Queue task cancelled: {uuid}"),
                        serde_json::json!({"entry_id": entry_id, "source": "bridge"}),
                    );
                }
                return cancelled;
            }
        }
        false
    }

    pub fn queue_retry(&self, entry_id: String) -> bool {
        if let Ok(uuid) = Uuid::parse_str(&entry_id) {
            if let Ok(mut s) = self.state.lock() {
                let ws_id = s.agent_queue.get(uuid)
                    .and_then(|e| e.workspace_id)
                    .unwrap_or(Uuid::nil());
                let retried = s.agent_queue.retry(uuid);
                if retried {
                    s.audit_log.log(
                        ws_id,
                        None,
                        AuditEventType::QueueTaskSubmitted,
                        CoreAuditSeverity::Info,
                        format!("Queue task retried: {uuid}"),
                        serde_json::json!({"entry_id": entry_id, "source": "bridge_retry"}),
                    );
                }
                return retried;
            }
        }
        false
    }

    // ── Configuration ──

    pub fn config_get(&self, key: String) -> Option<String> {
        let s = self.state.lock().ok()?;
        s.config.get(&key).map(|v| v.to_string())
    }

    pub fn config_set(&self, key: String, value: String) {
        if let Ok(mut s) = self.state.lock() {
            s.config.set(&key, &value);
            if let Some(cb) = &s.ui_callback {
                cb.config_changed();
            }
        }
    }

    pub fn config_all(&self) -> Vec<ConfigEntry> {
        let s = self.state.lock().unwrap();
        s.config.all().iter().map(|(k, v)| ConfigEntry {
            key: k.clone(),
            value: v.clone(),
        }).collect()
    }

    pub fn config_font_family(&self) -> String {
        let s = self.state.lock().unwrap();
        s.config.font_family().to_string()
    }

    pub fn config_font_size(&self) -> f64 {
        let s = self.state.lock().unwrap();
        s.config.font_size()
    }

    // ── Queue sandbox ──

    pub fn queue_sandbox_status(&self) -> Option<SandboxInfo> {
        let s = self.state.lock().ok()?;
        Some(sandbox_to_info(&s.agent_queue.sandbox_policy))
    }

    pub fn queue_sandbox_enable(&self) -> Result<(), BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        if !s.agent_queue.sandbox_policy.enabled {
            let base_dir = match s.config.get("queue-working-dir") {
                Some(dir) => PathBuf::from(dir),
                None => {
                    let home = std::env::var("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|_| PathBuf::from("/tmp"));
                    home.join("thane-tasks")
                }
            };
            s.agent_queue.sandbox_policy = SandboxPolicy::confined_to(base_dir);
        }
        s.agent_queue.sandbox_policy.enabled = true;
        Ok(())
    }

    pub fn queue_sandbox_disable(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.agent_queue.sandbox_policy.enabled = false;
        }
    }

    pub fn queue_sandbox_set_enforcement(&self, level: BridgeEnforcementLevel) -> Result<(), BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        s.agent_queue.sandbox_policy.enforcement = CoreEnforcement::from(level);
        Ok(())
    }

    pub fn queue_sandbox_set_network(&self, allow: bool) -> Result<(), BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        s.agent_queue.sandbox_policy.allow_network = allow;
        Ok(())
    }

    // ── Sandbox ──

    pub fn sandbox_status(&self, workspace_id: String) -> Option<SandboxInfo> {
        let uuid = Uuid::parse_str(&workspace_id).ok()?;
        let s = self.state.lock().ok()?;
        let ws = s.workspace_mgr.get(uuid)?;
        Some(sandbox_to_info(&ws.sandbox_policy))
    }

    pub fn sandbox_enable(&self, workspace_id: String) -> Result<(), BridgeError> {
        let uuid = parse_uuid(&workspace_id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.get_mut(uuid)
            .ok_or(BridgeError::WorkspaceNotFound)?;
        if !ws.sandbox_policy.enabled {
            // Initialize a fresh confined policy based on the workspace CWD
            ws.sandbox_policy = SandboxPolicy::confined_to(std::path::PathBuf::from(&ws.cwd));
        }
        ws.sandbox_policy.enabled = true;
        Ok(())
    }

    pub fn sandbox_disable(&self, workspace_id: String) {
        if let Ok(uuid) = Uuid::parse_str(&workspace_id) {
            if let Ok(mut s) = self.state.lock() {
                if let Some(ws) = s.workspace_mgr.get_mut(uuid) {
                    ws.sandbox_policy.enabled = false;
                }
            }
        }
    }

    pub fn sandbox_set_enforcement(&self, workspace_id: String, level: BridgeEnforcementLevel) -> Result<(), BridgeError> {
        let uuid = parse_uuid(&workspace_id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.get_mut(uuid)
            .ok_or(BridgeError::WorkspaceNotFound)?;
        ws.sandbox_policy.enforcement = CoreEnforcement::from(level);
        Ok(())
    }

    pub fn sandbox_set_network(&self, workspace_id: String, allow: bool) -> Result<(), BridgeError> {
        let uuid = parse_uuid(&workspace_id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.get_mut(uuid)
            .ok_or(BridgeError::WorkspaceNotFound)?;
        ws.sandbox_policy.allow_network = allow;
        Ok(())
    }

    pub fn sandbox_allow_path(&self, workspace_id: String, path: String, writable: bool) -> Result<(), BridgeError> {
        let uuid = parse_uuid(&workspace_id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.get_mut(uuid)
            .ok_or(BridgeError::WorkspaceNotFound)?;
        let path_buf = PathBuf::from(&path);
        if writable {
            ws.sandbox_policy.read_write_paths.push(path_buf);
        } else {
            ws.sandbox_policy.read_only_paths.push(path_buf);
        }
        Ok(())
    }

    pub fn sandbox_deny_path(&self, workspace_id: String, path: String) -> Result<(), BridgeError> {
        let uuid = parse_uuid(&workspace_id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let ws = s.workspace_mgr.get_mut(uuid)
            .ok_or(BridgeError::WorkspaceNotFound)?;
        ws.sandbox_policy.denied_paths.push(PathBuf::from(&path));
        Ok(())
    }

    pub fn sandbox_get_command(&self, workspace_id: String, shell: String) -> Option<SandboxCommand> {
        #[cfg(target_os = "macos")]
        {
            let uuid = Uuid::parse_str(&workspace_id).ok()?;
            let s = self.state.lock().ok()?;
            let ws = s.workspace_mgr.get(uuid)?;

            let (executable, args, extra_env) =
                thane_platform::sandbox_macos::generate_sandbox_command(&ws.sandbox_policy, &shell)?;

            Some(SandboxCommand {
                executable,
                args,
                extra_env,
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (workspace_id, shell);
            None
        }
    }

    // ── Session persistence ──

    pub fn save_session(&self) -> Result<(), BridgeError> {
        let s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let workspaces: Vec<_> = s.workspace_mgr.list().iter().map(|ws| {
            thane_core::session::WorkspaceSnapshot {
                id: ws.id,
                title: ws.title.clone(),
                cwd: ws.cwd.clone(),
                split_tree: ws.split_tree.clone(),
                panels: ws.panels.values().map(|p| {
                    thane_core::session::PanelSnapshot {
                        info: p.clone(),
                        scrollback: None,
                        url: if p.panel_type == CorePanelType::Browser {
                            Some(p.location.clone())
                        } else {
                            None
                        },
                    }
                }).collect(),
                focused_pane_id: Some(ws.focused_pane_id),
                tag: ws.tag.clone(),
                sandbox_policy: ws.sandbox_policy.clone(),
            }
        }).collect();

        let active_id = s.workspace_mgr.active().map(|ws| ws.id);
        let snapshot = AppSnapshot::new(workspaces, active_id);
        s.session_store.save(&snapshot)
            .map_err(|e| BridgeError::PersistenceError(e.to_string()))
    }

    pub fn restore_session(&self) -> Result<SessionInfo, BridgeError> {
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        match s.session_store.load() {
            Ok(Some(snapshot)) => {
                let mut count = 0u64;
                for ws_snap in &snapshot.workspaces {
                    let ws = thane_core::workspace::Workspace::restore_from_snapshot(ws_snap);
                    s.workspace_mgr.add(ws);
                    count += 1;
                }
                if let Some(active_id) = snapshot.active_workspace_id {
                    s.workspace_mgr.select_by_id(active_id);
                }
                Ok(SessionInfo {
                    restored: count > 0,
                    workspace_count: count,
                })
            }
            Ok(None) => Ok(SessionInfo {
                restored: false,
                workspace_count: 0,
            }),
            Err(e) => Err(BridgeError::PersistenceError(e.to_string())),
        }
    }

    // ── Agent detection ──

    /// Check if any of the given shell PIDs have an agent running as a child process.
    /// Returns "active:<agent_name>" if found, or "idle" if no agent detected.
    pub fn detect_agent_for_pids(&self, pids: Vec<i32>) -> String {
        use thane_core::agent::detect_agent_for_pid;
        use thane_core::sidebar::AgentStatus;

        for pid in pids {
            let detection = detect_agent_for_pid(Some(pid));
            if detection.status == AgentStatus::Active {
                if let Some(name) = detection.agent_name {
                    return format!("active:{name}");
                }
                return "active".to_string();
            }
        }
        "idle".to_string()
    }

    // ── Cost & token limits ──

    /// Get the current plan name and token limit utilization.
    /// Reads credentials from ~/.claude/.credentials.json and fetches
    /// usage data from the Anthropic OAuth API.
    pub fn get_token_limits(&self) -> TokenLimitsDTO {
        use thane_core::cost_tracker::{CostDisplayMode, Plan, read_oauth_token, fetch_oauth_usage, TokenLimitInfo};

        let plan_config = self.state.lock().ok()
            .and_then(|s| s.config.get("plan").map(|v| v.to_string()));
        let plan = Plan::detect(plan_config.as_deref());
        let plan_name = format!("{:?}", plan);

        let limit_info = read_oauth_token().and_then(|token| {
            let response = fetch_oauth_usage(&token)?;
            Some(TokenLimitInfo::from_oauth(plan.clone(), &response))
        });

        let display_mode_str = |mode: CostDisplayMode| match mode {
            CostDisplayMode::Utilization => "utilization".to_string(),
            CostDisplayMode::Dollar => "dollar".to_string(),
        };

        if let Some(info) = limit_info {
            let mode = info.display_mode();
            TokenLimitsDTO {
                plan_name,
                has_caps: info.has_caps,
                display_mode: display_mode_str(mode),
                five_hour_utilization: info.five_hour.as_ref().map(|w| w.utilization),
                five_hour_resets_at: info.five_hour.as_ref().map(|w| w.resets_at.to_rfc3339()),
                seven_day_utilization: info.seven_day.as_ref().map(|w| w.utilization),
                seven_day_resets_at: info.seven_day.as_ref().map(|w| w.resets_at.to_rfc3339()),
            }
        } else {
            TokenLimitsDTO {
                plan_name,
                has_caps: plan.has_caps(),
                display_mode: "dollar".to_string(),
                five_hour_utilization: None,
                five_hour_resets_at: None,
                seven_day_utilization: None,
                seven_day_resets_at: None,
            }
        }
    }

    /// Get project cost summary for the active workspace's CWD.
    ///
    /// Includes plan-aware display mode and utilization data so the macOS frontend
    /// can show the right primary metric without a separate `getTokenLimits()` call.
    pub fn get_project_cost(&self) -> ProjectCostDTO {
        use thane_core::cost_tracker::{CostDisplayMode, Plan, read_oauth_token, fetch_oauth_usage, TokenLimitInfo};

        let cwd = self.state.lock().ok()
            .and_then(|s| s.workspace_mgr.active().map(|ws| ws.cwd.clone()))
            .unwrap_or_default();

        if cwd.is_empty() {
            return ProjectCostDTO::default();
        }

        let summary = thane_core::cost_tracker::CostTracker::for_project_detailed(&cwd, None);

        // Resolve plan + utilization for display mode.
        let plan_config = self.state.lock().ok()
            .and_then(|s| s.config.get("plan").map(|v| v.to_string()));
        let plan = Plan::detect(plan_config.as_deref());

        let limit_info = read_oauth_token().and_then(|token| {
            let response = fetch_oauth_usage(&token)?;
            Some(TokenLimitInfo::from_oauth(plan.clone(), &response))
        });

        let (display_mode, five_hour_util, seven_day_util) = if let Some(ref info) = limit_info {
            let mode = match info.display_mode() {
                CostDisplayMode::Utilization => "utilization",
                CostDisplayMode::Dollar => "dollar",
            };
            (
                mode.to_string(),
                info.five_hour.as_ref().map(|w| w.utilization),
                info.seven_day.as_ref().map(|w| w.utilization),
            )
        } else {
            ("dollar".to_string(), None, None)
        };

        ProjectCostDTO {
            session_cost_usd: summary.current_session.estimated_cost_usd,
            session_input_tokens: summary.current_session.input_tokens,
            session_output_tokens: summary.current_session.output_tokens,
            session_cache_read_tokens: summary.current_session.cache_read_tokens,
            session_cache_write_tokens: summary.current_session.cache_write_tokens,
            alltime_cost_usd: summary.all_time.estimated_cost_usd,
            alltime_input_tokens: summary.all_time.input_tokens,
            alltime_output_tokens: summary.all_time.output_tokens,
            alltime_cache_read_tokens: summary.all_time.cache_read_tokens,
            alltime_cache_write_tokens: summary.all_time.cache_write_tokens,
            session_count: summary.sessions.len() as u64,
            plan_name: plan.display_name().to_string(),
            display_mode,
            five_hour_utilization: five_hour_util,
            seven_day_utilization: seven_day_util,
        }
    }

    // ── IPC server lifecycle ──

    /// Return the platform socket path (e.g. ~/Library/Application Support/thane/run/thane.sock).
    pub fn socket_path(&self) -> String {
        #[cfg(target_os = "macos")]
        let dirs = thane_platform::MacosDirs;
        #[cfg(target_os = "linux")]
        let dirs = thane_platform::LinuxDirs;
        dirs.socket_path().to_string_lossy().into_owned()
    }

    pub fn start_ipc_server(&self) -> Result<(), BridgeError> {
        #[cfg(target_os = "macos")]
        let dirs = thane_platform::MacosDirs;
        #[cfg(target_os = "linux")]
        let dirs = thane_platform::LinuxDirs;

        let socket_path = dirs.socket_path();

        // Ensure the parent directory exists.
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| BridgeError::IpcError(format!("Failed to create runtime dir: {e}")))?;
        }

        // Remove stale socket if it exists.
        let _ = std::fs::remove_file(&socket_path);

        // Set the env var so child processes (terminals) can find us.
        // Safety: called once at startup before other threads read this var.
        unsafe {
            std::env::set_var("THANE_SOCKET_PATH", socket_path.to_string_lossy().as_ref());
        }

        let handler = build_bridge_rpc_handler(self.state.clone());
        let access_mode = thane_ipc::auth::AccessMode::Open;

        let path = socket_path.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime for IPC");

            rt.block_on(async move {
                if let Err(e) =
                    thane_ipc::server::start_server(&path, handler, access_mode).await
                {
                    tracing::error!("IPC server error: {e}");
                }
            });
        });

        tracing::info!("IPC server starting on {}", socket_path.display());
        Ok(())
    }

    pub fn stop_ipc_server(&self) {
        #[cfg(target_os = "macos")]
        let dirs = thane_platform::MacosDirs;
        #[cfg(target_os = "linux")]
        let dirs = thane_platform::LinuxDirs;
        let _ = std::fs::remove_file(dirs.socket_path());
        tracing::info!("IPC server stopped");
    }

    // ── Browser control ──

    pub fn browser_navigate(&self, panel_id: String, url: String) -> Result<(), BridgeError> {
        // Browser navigation is handled by the Swift UI layer directly.
        // This stub exists for CLI/RPC parity.
        tracing::info!("browser_navigate: panel={panel_id} url={url}");
        Ok(())
    }

    pub fn browser_eval_js(&self, panel_id: String, code: String) -> Result<String, BridgeError> {
        tracing::info!("browser_eval_js: panel={panel_id}");
        let _ = code;
        Err(BridgeError::InvalidOperation("Browser JS evaluation is handled by the Swift UI layer. Use WKWebView directly.".into()))
    }

    pub fn browser_screenshot(&self, panel_id: String) -> Result<String, BridgeError> {
        tracing::info!("browser_screenshot: panel={panel_id}");
        Err(BridgeError::InvalidOperation("Browser screenshot is handled by the Swift UI layer.".into()))
    }

    pub fn browser_get_accessibility_tree(&self, panel_id: String) -> Result<String, BridgeError> {
        tracing::info!("browser_get_accessibility_tree: panel={panel_id}");
        Err(BridgeError::InvalidOperation("Accessibility tree is handled by the Swift UI layer.".into()))
    }

    pub fn browser_click_element(&self, panel_id: String, selector: String) -> Result<(), BridgeError> {
        tracing::info!("browser_click_element: panel={panel_id} selector={selector}");
        Err(BridgeError::InvalidOperation("Element clicking is handled by the Swift UI layer.".into()))
    }

    pub fn browser_type_text(&self, panel_id: String, text: String) -> Result<(), BridgeError> {
        tracing::info!("browser_type_text: panel={panel_id}");
        let _ = text;
        Err(BridgeError::InvalidOperation("Text typing is handled by the Swift UI layer.".into()))
    }

    // ── Workspace history ──

    pub fn history_list(&self) -> Vec<ClosedWorkspaceInfo> {
        let s = self.state.lock().unwrap();
        s.workspace_history.list().iter().map(closed_ws_to_info).collect()
    }

    pub fn history_reopen(&self, id: String) -> Result<WorkspaceInfo, BridgeError> {
        let uuid = parse_uuid(&id)?;
        let mut s = self.state.lock().map_err(|e| BridgeError::Internal(e.to_string()))?;
        let record = s.workspace_history.remove(uuid)
            .ok_or(BridgeError::WorkspaceNotFound)?;
        let tag = record.tag.clone();
        let ws = s.workspace_mgr.create(&record.title, &record.cwd);
        let ws_id = ws.id;
        let info = workspace_to_info(ws);
        if let Some(tag) = tag {
            if let Some(w) = s.workspace_mgr.get_mut(ws_id) {
                w.tag = Some(tag);
            }
        }
        if let Err(e) = s.history_store.save(&s.workspace_history) {
            tracing::warn!("Failed to save history after reopen: {e}");
        }
        if let Some(cb) = &s.ui_callback {
            cb.workspace_list_changed();
            cb.workspace_changed(info.id.clone());
        }
        Ok(info)
    }

    pub fn history_clear(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.workspace_history.clear();
            if let Err(e) = s.history_store.save(&s.workspace_history) {
                tracing::warn!("Failed to save history after clear: {e}");
            }
        }
    }

    // ── Callback registration ──

    pub fn set_ui_callback(&self, callback: Box<dyn UiCallback>) {
        if let Ok(mut s) = self.state.lock() {
            s.ui_callback = Some(callback);
        }
    }
}

// ─── Helper functions ──────────────────────────────────────

fn parse_uuid(s: &str) -> Result<Uuid, BridgeError> {
    Uuid::parse_str(s).map_err(|e| BridgeError::InvalidOperation(format!("Invalid UUID: {e}")))
}

fn parse_panel_id(s: &str) -> Result<thane_core::panel::PanelId, BridgeError> {
    parse_uuid(s)
}

fn workspace_to_info(ws: &thane_core::workspace::Workspace) -> WorkspaceInfo {
    WorkspaceInfo {
        id: ws.id.to_string(),
        title: ws.title.clone(),
        cwd: ws.cwd.clone(),
        tag: ws.tag.clone(),
        pane_count: ws.pane_count() as u64,
        panel_count: ws.panels.len() as u64,
        unread_notifications: ws.notifications.unread_count() as u64,
    }
}

fn notification_to_info(n: &thane_core::notification::Notification) -> NotificationInfo {
    NotificationInfo {
        id: n.id.to_string(),
        panel_id: n.panel_id.to_string(),
        title: n.title.clone(),
        body: n.body.clone(),
        urgency: n.urgency.into(),
        timestamp: n.timestamp.to_rfc3339(),
        read: n.read,
    }
}

fn queue_entry_to_info(e: &thane_core::agent_queue::QueueEntry) -> QueueEntryInfo {
    QueueEntryInfo {
        id: e.id.to_string(),
        content: e.content.clone(),
        workspace_id: e.workspace_id.map(|id| id.to_string()),
        priority: e.priority,
        status: e.status.clone().into(),
        created_at: e.created_at.to_rfc3339(),
        started_at: e.started_at.map(|t| t.to_rfc3339()),
        completed_at: e.completed_at.map(|t| t.to_rfc3339()),
        error: e.error.clone(),
        input_tokens: e.tokens_used.input_tokens,
        output_tokens: e.tokens_used.output_tokens,
        estimated_cost_usd: e.tokens_used.estimated_cost_usd,
        model: e.tokens_used.model.clone(),
        depends_on: e.depends_on.map(|id| id.to_string()),
    }
}

fn closed_ws_to_info(r: &ClosedWorkspaceRecord) -> ClosedWorkspaceInfo {
    ClosedWorkspaceInfo {
        id: r.original_id.to_string(),
        title: r.title.clone(),
        cwd: r.cwd.clone(),
        tag: r.tag.clone(),
        closed_at: r.closed_at.to_rfc3339(),
    }
}

fn sandbox_to_info(policy: &SandboxPolicy) -> SandboxInfo {
    SandboxInfo {
        enabled: policy.enabled,
        root_dir: policy.root_dir.to_string_lossy().to_string(),
        read_only_paths: policy.read_only_paths.iter()
            .map(|p| p.to_string_lossy().to_string()).collect(),
        read_write_paths: policy.read_write_paths.iter()
            .map(|p| p.to_string_lossy().to_string()).collect(),
        denied_paths: policy.denied_paths.iter()
            .map(|p| p.to_string_lossy().to_string()).collect(),
        allow_network: policy.allow_network,
        max_open_files: policy.max_open_files,
        max_write_bytes: policy.max_write_bytes,
        max_cpu_seconds: policy.cpu_time_limit,
        enforcement: policy.enforcement.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Create a ThaneBridge backed by a unique temp config directory per test.
    fn test_bridge() -> ThaneBridge {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "thane-bridge-test-{}-{id}", std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let config_path = dir.join("config");
        std::fs::write(&config_path, "font-family = TestMono\nfont-size = 16\n").unwrap();
        ThaneBridge::new(Some(config_path.to_string_lossy().to_string())).unwrap()
    }

    // ── Constructor ──

    #[test]
    fn test_new_default_config() {
        let bridge = ThaneBridge::new(None).unwrap();
        assert!(bridge.list_workspaces().is_empty());
    }

    #[test]
    fn test_new_custom_config() {
        let bridge = test_bridge();
        assert_eq!(bridge.config_font_family(), "TestMono");
        assert_eq!(bridge.config_font_size(), 16.0);
    }

    #[test]
    fn test_new_invalid_config_path() {
        let result = ThaneBridge::new(Some("/nonexistent/path/config".to_string()));
        assert!(result.is_err());
    }

    // ── Workspace CRUD ──

    #[test]
    fn test_create_workspace() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Project".into(), "/tmp".into()).unwrap();
        assert_eq!(ws.title, "Project");
        assert_eq!(ws.cwd, "/tmp");
        assert_eq!(ws.pane_count, 1);
        assert_eq!(ws.panel_count, 1);
    }

    #[test]
    fn test_list_workspaces() {
        let bridge = test_bridge();
        bridge.create_workspace("WS1".into(), "/tmp".into()).unwrap();
        bridge.create_workspace("WS2".into(), "/tmp".into()).unwrap();
        assert_eq!(bridge.list_workspaces().len(), 2);
    }

    #[test]
    fn test_active_workspace() {
        let bridge = test_bridge();
        assert!(bridge.active_workspace().is_none());
        bridge.create_workspace("Active".into(), "/tmp".into()).unwrap();
        let active = bridge.active_workspace().unwrap();
        assert_eq!(active.title, "Active");
    }

    #[test]
    fn test_select_workspace() {
        let bridge = test_bridge();
        let ws1 = bridge.create_workspace("WS1".into(), "/tmp".into()).unwrap();
        let ws2 = bridge.create_workspace("WS2".into(), "/tmp".into()).unwrap();
        assert_eq!(bridge.active_workspace().unwrap().id, ws2.id);

        bridge.select_workspace(ws1.id.clone()).unwrap();
        assert_eq!(bridge.active_workspace().unwrap().id, ws1.id);
    }

    #[test]
    fn test_select_workspace_invalid_uuid() {
        let bridge = test_bridge();
        let result = bridge.select_workspace("not-a-uuid".into());
        assert!(result.is_err());
    }

    #[test]
    fn test_rename_workspace() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Old".into(), "/tmp".into()).unwrap();
        bridge.rename_workspace(ws.id.clone(), "New".into()).unwrap();
        assert_eq!(bridge.active_workspace().unwrap().title, "New");
    }

    #[test]
    fn test_rename_nonexistent_workspace() {
        let bridge = test_bridge();
        let result = bridge.rename_workspace(Uuid::new_v4().to_string(), "X".into());
        assert!(matches!(result, Err(BridgeError::WorkspaceNotFound)));
    }

    #[test]
    fn test_close_workspace() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Doomed".into(), "/tmp".into()).unwrap();
        assert!(bridge.close_workspace(ws.id).unwrap());
        assert!(bridge.list_workspaces().is_empty());
    }

    #[test]
    fn test_close_nonexistent_workspace() {
        let bridge = test_bridge();
        let result = bridge.close_workspace(Uuid::new_v4().to_string()).unwrap();
        assert!(!result);
    }

    // ── Split operations ──

    #[test]
    fn test_split_terminal_horizontal() {
        let bridge = test_bridge();
        bridge.create_workspace("Split".into(), "/tmp".into()).unwrap();
        let result = bridge.split_terminal(SplitOrientation::Horizontal).unwrap();
        assert!(!result.pane_id.is_empty());
        assert!(!result.panel_id.is_empty());
        assert_eq!(bridge.active_workspace().unwrap().pane_count, 2);
    }

    #[test]
    fn test_split_terminal_vertical() {
        let bridge = test_bridge();
        bridge.create_workspace("Split".into(), "/tmp".into()).unwrap();
        bridge.split_terminal(SplitOrientation::Vertical).unwrap();
        assert_eq!(bridge.active_workspace().unwrap().pane_count, 2);
    }

    #[test]
    fn test_split_browser() {
        let bridge = test_bridge();
        bridge.create_workspace("Browser".into(), "/tmp".into()).unwrap();
        let result = bridge.split_browser("https://example.com".into(), SplitOrientation::Horizontal).unwrap();
        assert!(!result.panel_id.is_empty());
        assert_eq!(bridge.active_workspace().unwrap().pane_count, 2);
    }

    #[test]
    fn test_split_without_workspace_fails() {
        let bridge = test_bridge();
        let result = bridge.split_terminal(SplitOrientation::Horizontal);
        assert!(matches!(result, Err(BridgeError::WorkspaceNotFound)));
    }

    #[test]
    fn test_close_pane() {
        let bridge = test_bridge();
        bridge.create_workspace("Pane".into(), "/tmp".into()).unwrap();
        bridge.split_terminal(SplitOrientation::Horizontal).unwrap();
        assert_eq!(bridge.active_workspace().unwrap().pane_count, 2);
        bridge.close_pane().unwrap();
        assert_eq!(bridge.active_workspace().unwrap().pane_count, 1);
    }

    #[test]
    fn test_focus_direction() {
        let bridge = test_bridge();
        bridge.create_workspace("Focus".into(), "/tmp".into()).unwrap();
        bridge.split_terminal(SplitOrientation::Horizontal).unwrap();
        assert!(bridge.focus_direction("next".into()).is_ok());
        assert!(bridge.focus_direction("prev".into()).is_ok());
    }

    #[test]
    fn test_focus_direction_invalid() {
        let bridge = test_bridge();
        bridge.create_workspace("Focus".into(), "/tmp".into()).unwrap();
        let result = bridge.focus_direction("left".into());
        assert!(matches!(result, Err(BridgeError::InvalidOperation(_))));
    }

    // ── Panel management ──

    #[test]
    fn test_add_browser_panel() {
        let bridge = test_bridge();
        bridge.create_workspace("Panels".into(), "/tmp".into()).unwrap();
        let panel_id = bridge.add_browser_panel("https://example.com".into()).unwrap();
        assert!(!panel_id.is_empty());
        // Should have 2 panels now (initial terminal + added browser).
        assert_eq!(bridge.active_workspace().unwrap().panel_count, 2);
    }

    #[test]
    fn test_close_panel() {
        let bridge = test_bridge();
        bridge.create_workspace("Panels".into(), "/tmp".into()).unwrap();
        // Split first to create a second pane, then add a browser panel to it.
        let split = bridge.split_terminal(SplitOrientation::Horizontal).unwrap();
        let browser_id = bridge.add_browser_panel("https://example.com".into()).unwrap();
        let initial_count = bridge.active_workspace().unwrap().panel_count;
        // Close the browser panel (not the last panel in pane).
        bridge.close_panel(browser_id).unwrap();
        let after_count = bridge.active_workspace().unwrap().panel_count;
        assert!(after_count < initial_count);
    }

    #[test]
    fn test_list_panels() {
        let bridge = test_bridge();
        bridge.create_workspace("Panels".into(), "/tmp".into()).unwrap();
        let panels = bridge.list_panels();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].panel_type, BridgePanelType::Terminal);
    }

    #[test]
    fn test_focused_panel() {
        let bridge = test_bridge();
        bridge.create_workspace("Focus".into(), "/tmp".into()).unwrap();
        let panel = bridge.focused_panel().unwrap();
        assert_eq!(panel.panel_type, BridgePanelType::Terminal);
    }

    #[test]
    fn test_select_panel() {
        let bridge = test_bridge();
        bridge.create_workspace("Select".into(), "/tmp".into()).unwrap();
        let panel_id = bridge.add_browser_panel("https://example.com".into()).unwrap();
        assert!(bridge.select_panel(panel_id));
    }

    // ── Notifications ──

    #[test]
    fn test_notifications_empty_by_default() {
        let bridge = test_bridge();
        bridge.create_workspace("Notif".into(), "/tmp".into()).unwrap();
        assert!(bridge.list_notifications(None).is_empty());
        assert_eq!(bridge.unread_notification_count(), 0);
    }

    #[test]
    fn test_clear_notifications() {
        let bridge = test_bridge();
        bridge.create_workspace("Notif".into(), "/tmp".into()).unwrap();
        bridge.clear_notifications();
        assert!(bridge.list_notifications(None).is_empty());
    }

    // ── Audit log ──

    #[test]
    fn test_audit_log_empty_by_default() {
        let bridge = test_bridge();
        assert!(bridge.list_audit_events(None).is_empty());
    }

    #[test]
    fn test_audit_log_populated_after_queue_submit() {
        let bridge = test_bridge();
        bridge.queue_submit("Test task".into(), None, 0);
        let events = bridge.list_audit_events(None);
        assert!(!events.is_empty());
        assert!(events[0].description.contains("Test task"));
    }

    #[test]
    fn test_audit_log_severity_filter() {
        let bridge = test_bridge();
        bridge.queue_submit("task".into(), None, 0);
        // Queue submit logs Info severity — filtering by Warning+ should exclude it.
        let warning_events = bridge.list_audit_events(Some(BridgeAuditSeverity::Warning));
        assert!(warning_events.is_empty());
    }

    #[test]
    fn test_export_audit_json() {
        let bridge = test_bridge();
        bridge.queue_submit("task".into(), None, 0);
        let json = bridge.export_audit_json();
        assert!(json.starts_with('['));
        assert!(json.contains("task"));
    }

    #[test]
    fn test_clear_audit_log() {
        let bridge = test_bridge();
        // Submit multiple tasks to accumulate audit events.
        bridge.queue_submit("task1".into(), None, 0);
        bridge.queue_submit("task2".into(), None, 0);
        bridge.queue_submit("task3".into(), None, 0);
        let before = bridge.list_audit_events(None).len();
        assert!(before >= 3);
        bridge.clear_audit_log();
        // clear() logs an "AuditCleared" event and keeps only that one.
        let after = bridge.list_audit_events(None).len();
        assert_eq!(after, 1);
        assert!(after < before);
    }

    // ── Agent queue ──

    #[test]
    fn test_queue_submit_and_list() {
        let bridge = test_bridge();
        let id = bridge.queue_submit("Do something".into(), None, 0);
        assert!(!id.is_empty());
        let entries = bridge.queue_list();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "Do something");
        assert_eq!(entries[0].status, BridgeQueueEntryStatus::Queued);
    }

    #[test]
    fn test_queue_submit_with_workspace() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Queue".into(), "/tmp".into()).unwrap();
        let id = bridge.queue_submit("task".into(), Some(ws.id.clone()), 5);
        let entry = bridge.queue_status(id).unwrap();
        assert_eq!(entry.workspace_id.unwrap(), ws.id);
        assert_eq!(entry.priority, 5);
    }

    #[test]
    fn test_queue_cancel() {
        let bridge = test_bridge();
        let id = bridge.queue_submit("cancel me".into(), None, 0);
        assert!(bridge.queue_cancel(id.clone()));
        let entry = bridge.queue_status(id).unwrap();
        assert_eq!(entry.status, BridgeQueueEntryStatus::Cancelled);
    }

    #[test]
    fn test_queue_status_nonexistent() {
        let bridge = test_bridge();
        assert!(bridge.queue_status(Uuid::new_v4().to_string()).is_none());
    }

    #[test]
    fn test_queue_submit_with_depends() {
        let bridge = test_bridge();
        let phase1_id = bridge.queue_submit("Phase 1".into(), None, 0);
        let phase2_id = bridge.queue_submit_with_depends(
            "Phase 2".into(),
            None,
            0,
            Some(phase1_id.clone()),
        );
        assert!(!phase2_id.is_empty());

        // Verify depends_on is surfaced through queue_status.
        let entry = bridge.queue_status(phase2_id).unwrap();
        assert_eq!(entry.depends_on.as_deref(), Some(phase1_id.as_str()));

        // Verify depends_on is surfaced through queue_list.
        let entries = bridge.queue_list();
        let phase2_entry = entries.iter().find(|e| e.depends_on.is_some()).unwrap();
        assert_eq!(phase2_entry.depends_on.as_deref(), Some(phase1_id.as_str()));
    }

    #[test]
    fn test_queue_submit_with_depends_none() {
        let bridge = test_bridge();
        let id = bridge.queue_submit_with_depends("No dep".into(), None, 0, None);
        let entry = bridge.queue_status(id).unwrap();
        assert!(entry.depends_on.is_none());
    }

    // ── Configuration ──

    #[test]
    fn test_config_get_set() {
        let bridge = test_bridge();
        bridge.config_set("theme".into(), "dark".into());
        assert_eq!(bridge.config_get("theme".into()).unwrap(), "dark");
    }

    #[test]
    fn test_config_get_missing() {
        let bridge = test_bridge();
        assert!(bridge.config_get("nonexistent-key".into()).is_none());
    }

    #[test]
    fn test_config_all() {
        let bridge = test_bridge();
        bridge.config_set("custom-key".into(), "custom-val".into());
        let all = bridge.config_all();
        assert!(all.iter().any(|e| e.key == "custom-key" && e.value == "custom-val"));
    }

    // ── Sandbox ──

    #[test]
    fn test_sandbox_disabled_by_default() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Sandbox".into(), "/tmp".into()).unwrap();
        let status = bridge.sandbox_status(ws.id).unwrap();
        assert!(!status.enabled);
    }

    #[test]
    fn test_sandbox_enable_disable() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Sandbox".into(), "/tmp/project".into()).unwrap();
        bridge.sandbox_enable(ws.id.clone()).unwrap();
        assert!(bridge.sandbox_status(ws.id.clone()).unwrap().enabled);

        bridge.sandbox_disable(ws.id.clone());
        assert!(!bridge.sandbox_status(ws.id).unwrap().enabled);
    }

    #[test]
    fn test_sandbox_set_enforcement() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Sandbox".into(), "/tmp".into()).unwrap();
        bridge.sandbox_enable(ws.id.clone()).unwrap();
        bridge.sandbox_set_enforcement(ws.id.clone(), BridgeEnforcementLevel::Strict).unwrap();
        assert_eq!(bridge.sandbox_status(ws.id).unwrap().enforcement, BridgeEnforcementLevel::Strict);
    }

    #[test]
    fn test_sandbox_network_toggle() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Sandbox".into(), "/tmp".into()).unwrap();
        bridge.sandbox_enable(ws.id.clone()).unwrap();
        bridge.sandbox_set_network(ws.id.clone(), false).unwrap();
        assert!(!bridge.sandbox_status(ws.id).unwrap().allow_network);
    }

    #[test]
    fn test_sandbox_allow_path() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Sandbox".into(), "/tmp".into()).unwrap();
        bridge.sandbox_enable(ws.id.clone()).unwrap();
        bridge.sandbox_allow_path(ws.id.clone(), "/usr/lib".into(), false).unwrap();
        bridge.sandbox_allow_path(ws.id.clone(), "/tmp/out".into(), true).unwrap();
        let status = bridge.sandbox_status(ws.id).unwrap();
        assert!(status.read_only_paths.contains(&"/usr/lib".to_string()));
        assert!(status.read_write_paths.contains(&"/tmp/out".to_string()));
    }

    #[test]
    fn test_sandbox_deny_path() {
        let bridge = test_bridge();
        let ws = bridge.create_workspace("Sandbox".into(), "/tmp".into()).unwrap();
        bridge.sandbox_enable(ws.id.clone()).unwrap();
        bridge.sandbox_deny_path(ws.id.clone(), "/etc/passwd".into()).unwrap();
        let status = bridge.sandbox_status(ws.id).unwrap();
        assert!(status.denied_paths.contains(&"/etc/passwd".to_string()));
    }

    // ── Session persistence ──

    #[test]
    fn test_save_and_restore_session() {
        let bridge = test_bridge();
        bridge.create_workspace("Persist1".into(), "/tmp/a".into()).unwrap();
        bridge.create_workspace("Persist2".into(), "/tmp/b".into()).unwrap();
        bridge.save_session().unwrap();

        // Restore in the same bridge after clearing workspaces.
        // Close both, then restore.
        let ws_ids: Vec<_> = bridge.list_workspaces().iter().map(|w| w.id.clone()).collect();
        for id in ws_ids {
            bridge.close_workspace(id).unwrap();
        }
        assert!(bridge.list_workspaces().is_empty());

        let info = bridge.restore_session().unwrap();
        assert!(info.restored);
        assert_eq!(info.workspace_count, 2);
        assert_eq!(bridge.list_workspaces().len(), 2);
    }

    #[test]
    fn test_restore_session_does_not_error() {
        // The session store uses the platform default directory, so we can't
        // guarantee it's empty. Just verify restore doesn't return an error.
        let bridge = test_bridge();
        let info = bridge.restore_session().unwrap();
        // If there's a session file from other tests or real usage, this is fine.
        assert_eq!(info.restored, info.workspace_count > 0);
    }

    // ── Error mapping ──

    #[test]
    fn test_core_error_to_bridge_error() {
        let core_err = thane_core::error::CoreError::WorkspaceNotFound(Uuid::new_v4());
        let bridge_err: BridgeError = core_err.into();
        assert!(matches!(bridge_err, BridgeError::WorkspaceNotFound));

        let core_err = thane_core::error::CoreError::PanelNotFound(Uuid::new_v4());
        let bridge_err: BridgeError = core_err.into();
        assert!(matches!(bridge_err, BridgeError::PanelNotFound));

        let core_err = thane_core::error::CoreError::PaneNotFound(Uuid::new_v4());
        let bridge_err: BridgeError = core_err.into();
        assert!(matches!(bridge_err, BridgeError::PaneNotFound));
    }

    // ── Enum conversions ──

    #[test]
    fn test_orientation_conversion() {
        assert_eq!(Orientation::from(SplitOrientation::Horizontal), Orientation::Horizontal);
        assert_eq!(Orientation::from(SplitOrientation::Vertical), Orientation::Vertical);
    }

    #[test]
    fn test_enforcement_roundtrip() {
        let levels = [
            (BridgeEnforcementLevel::Permissive, CoreEnforcement::Permissive),
            (BridgeEnforcementLevel::Enforcing, CoreEnforcement::Enforcing),
            (BridgeEnforcementLevel::Strict, CoreEnforcement::Strict),
        ];
        for (bridge, core) in levels {
            assert_eq!(CoreEnforcement::from(bridge), core);
            assert_eq!(BridgeEnforcementLevel::from(core), bridge);
        }
    }

    #[test]
    fn test_severity_roundtrip() {
        let severities = [
            (BridgeAuditSeverity::Info, CoreAuditSeverity::Info),
            (BridgeAuditSeverity::Warning, CoreAuditSeverity::Warning),
            (BridgeAuditSeverity::Alert, CoreAuditSeverity::Alert),
            (BridgeAuditSeverity::Critical, CoreAuditSeverity::Critical),
        ];
        for (bridge, core) in severities {
            assert_eq!(CoreAuditSeverity::from(bridge), core);
            assert_eq!(BridgeAuditSeverity::from(core), bridge);
        }
    }

    #[test]
    fn test_queue_status_conversion() {
        assert_eq!(BridgeQueueEntryStatus::from(CoreQueueStatus::Queued), BridgeQueueEntryStatus::Queued);
        assert_eq!(BridgeQueueEntryStatus::from(CoreQueueStatus::Running), BridgeQueueEntryStatus::Running);
        assert_eq!(BridgeQueueEntryStatus::from(CoreQueueStatus::Completed), BridgeQueueEntryStatus::Completed);
        assert_eq!(BridgeQueueEntryStatus::from(CoreQueueStatus::Failed), BridgeQueueEntryStatus::Failed);
        assert_eq!(BridgeQueueEntryStatus::from(CoreQueueStatus::Cancelled), BridgeQueueEntryStatus::Cancelled);
        assert_eq!(BridgeQueueEntryStatus::from(CoreQueueStatus::PausedTokenLimit), BridgeQueueEntryStatus::PausedTokenLimit);
        assert_eq!(BridgeQueueEntryStatus::from(CoreQueueStatus::PausedByUser), BridgeQueueEntryStatus::PausedByUser);
    }

    // ── Callback dispatch ──

    #[derive(Clone)]
    struct CallbackFlags {
        workspace_changed: Arc<AtomicBool>,
        workspace_list_changed: Arc<AtomicBool>,
        config_changed: Arc<AtomicBool>,
        layout_changed: Arc<AtomicBool>,
    }

    impl CallbackFlags {
        fn new() -> Self {
            Self {
                workspace_changed: Arc::new(AtomicBool::new(false)),
                workspace_list_changed: Arc::new(AtomicBool::new(false)),
                config_changed: Arc::new(AtomicBool::new(false)),
                layout_changed: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    struct TestCallback {
        flags: CallbackFlags,
    }

    impl UiCallback for TestCallback {
        fn workspace_changed(&self, _active_id: String) {
            self.flags.workspace_changed.store(true, Ordering::SeqCst);
        }
        fn workspace_list_changed(&self) {
            self.flags.workspace_list_changed.store(true, Ordering::SeqCst);
        }
        fn notification_received(&self, _workspace_id: String, _title: String, _body: String) {}
        fn agent_status_changed(&self, _workspace_id: String, _active: bool) {}
        fn queue_entry_completed(&self, _entry_id: String, _success: bool) {}
        fn pane_layout_changed(&self, _workspace_id: String) {
            self.flags.layout_changed.store(true, Ordering::SeqCst);
        }
        fn config_changed(&self) {
            self.flags.config_changed.store(true, Ordering::SeqCst);
        }
    }

    fn setup_callback(bridge: &ThaneBridge) -> CallbackFlags {
        let flags = CallbackFlags::new();
        bridge.set_ui_callback(Box::new(TestCallback { flags: flags.clone() }));
        flags
    }

    #[test]
    fn test_callback_on_workspace_create() {
        let bridge = test_bridge();
        let flags = setup_callback(&bridge);
        bridge.create_workspace("CB".into(), "/tmp".into()).unwrap();
        assert!(flags.workspace_changed.load(Ordering::SeqCst));
        assert!(flags.workspace_list_changed.load(Ordering::SeqCst));
    }

    #[test]
    fn test_callback_on_config_set() {
        let bridge = test_bridge();
        let flags = setup_callback(&bridge);
        bridge.config_set("theme".into(), "dark".into());
        assert!(flags.config_changed.load(Ordering::SeqCst));
    }

    #[test]
    fn test_callback_on_split() {
        let bridge = test_bridge();
        bridge.create_workspace("Split".into(), "/tmp".into()).unwrap();
        let flags = setup_callback(&bridge);
        bridge.split_terminal(SplitOrientation::Horizontal).unwrap();
        assert!(flags.layout_changed.load(Ordering::SeqCst));
    }

    // ── Browser stubs ──

    #[test]
    fn test_browser_eval_js_returns_error() {
        let bridge = test_bridge();
        let result = bridge.browser_eval_js("some-id".into(), "alert(1)".into());
        assert!(matches!(result, Err(BridgeError::InvalidOperation(_))));
    }

    #[test]
    fn test_browser_navigate_succeeds() {
        let bridge = test_bridge();
        assert!(bridge.browser_navigate("id".into(), "https://example.com".into()).is_ok());
    }

    // ── Workspace history ──

    #[test]
    fn test_history_empty_by_default() {
        let bridge = test_bridge();
        assert!(bridge.history_list().is_empty());
    }

    #[test]
    fn test_history_clear() {
        let bridge = test_bridge();
        bridge.history_clear();
        assert!(bridge.history_list().is_empty());
    }
}

// ─── RPC handler for bridge IPC ────────────────────────────

use thane_ipc::client::{AsyncRpcHandler, RpcFuture};
use thane_rpc::methods::Method;
use thane_rpc::protocol::{RpcRequest, RpcResponse};

/// Build an async RPC handler that dispatches requests to the bridge state.
///
/// Unlike the GTK handler which needs glib main-loop dispatch, the bridge uses
/// `Arc<Mutex<BridgeState>>` so we can handle requests directly on the tokio thread.
fn build_bridge_rpc_handler(state: Arc<Mutex<BridgeState>>) -> AsyncRpcHandler {
    Arc::new(move |request: RpcRequest| -> RpcFuture {
        let state = state.clone();
        Box::pin(async move {
            let method = match Method::parse(&request.method) {
                Some(m) => m,
                None => return RpcResponse::method_not_found(request.id, &request.method),
            };

            let id = request.id.clone();
            let params = &request.params;

            match method {
                Method::Ping => {
                    RpcResponse::success(id, serde_json::json!({"pong": true}))
                }
                Method::GetVersion => {
                    RpcResponse::success(id, serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"),
                    }))
                }
                Method::GetConfig => {
                    let Ok(s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let all: std::collections::HashMap<String, String> = s.config.all()
                        .iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    RpcResponse::success(id, serde_json::json!(all))
                }
                Method::WorkspaceList => {
                    let Ok(s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let list: Vec<_> = s.workspace_mgr.list().iter()
                        .map(|ws| serde_json::json!({
                            "id": ws.id.to_string(),
                            "title": ws.title,
                            "cwd": ws.cwd,
                        }))
                        .collect();
                    RpcResponse::success(id, serde_json::json!(list))
                }
                Method::WorkspaceCreate => {
                    let title = params.get("title").and_then(|v| v.as_str()).unwrap_or("New");
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                    let cwd = params.get("cwd").and_then(|v| v.as_str()).unwrap_or(&home);
                    let Ok(mut s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let ws = s.workspace_mgr.create(title, cwd);
                    let ws_id = ws.id.to_string();
                    if let Some(cb) = &s.ui_callback {
                        cb.workspace_list_changed();
                        cb.workspace_changed(ws_id.clone());
                    }
                    RpcResponse::success(id, serde_json::json!({"id": ws_id}))
                }
                Method::WorkspaceSelect => {
                    let ws_id = match params.get("id").and_then(|v| v.as_str()) {
                        Some(id_str) => id_str.to_string(),
                        None => return RpcResponse::invalid_params(id, "Missing 'id' parameter"),
                    };
                    let Ok(mut s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let uuid = match uuid::Uuid::parse_str(&ws_id) {
                        Ok(u) => u,
                        Err(_) => return RpcResponse::invalid_params(id, "Invalid UUID"),
                    };
                    s.workspace_mgr.select_by_id(uuid);
                    if let Some(cb) = &s.ui_callback {
                        cb.workspace_changed(ws_id);
                    }
                    RpcResponse::success(id, serde_json::json!({"ok": true}))
                }
                Method::AgentQueueSubmit => {
                    let content = match params.get("content").and_then(|v| v.as_str()) {
                        Some(c) => c.to_string(),
                        None => return RpcResponse::invalid_params(id, "Missing 'content' parameter"),
                    };
                    let Ok(mut s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let entry_id = s.agent_queue.submit(content.clone(), None, 0);
                    if let Some(cb) = &s.ui_callback {
                        cb.queue_entry_completed(entry_id.to_string(), false);
                    }
                    RpcResponse::success(id, serde_json::json!({"id": entry_id.to_string()}))
                }
                Method::AgentQueueList => {
                    let Ok(s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let entries: Vec<_> = s.agent_queue.list().iter()
                        .map(|e| serde_json::json!({
                            "id": e.id.to_string(),
                            "content": e.content,
                            "status": format!("{:?}", e.status),
                        }))
                        .collect();
                    RpcResponse::success(id, serde_json::json!(entries))
                }
                Method::AgentQueueCancel => {
                    let entry_id = match params.get("id").and_then(|v| v.as_str()) {
                        Some(id_str) => id_str.to_string(),
                        None => return RpcResponse::invalid_params(id, "Missing 'id' parameter"),
                    };
                    let Ok(mut s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let uuid = match uuid::Uuid::parse_str(&entry_id) {
                        Ok(u) => u,
                        Err(_) => return RpcResponse::invalid_params(id, "Invalid UUID"),
                    };
                    s.agent_queue.cancel(uuid);
                    RpcResponse::success(id, serde_json::json!({"ok": true}))
                }
                Method::NotificationList => {
                    let Ok(s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    if let Some(ws) = s.workspace_mgr.active() {
                        let notes: Vec<_> = ws.notifications.all().iter()
                            .map(|n| serde_json::json!({
                                "title": n.title,
                                "body": n.body,
                            }))
                            .collect();
                        RpcResponse::success(id, serde_json::json!(notes))
                    } else {
                        RpcResponse::success(id, serde_json::json!([]))
                    }
                }
                Method::AuditList => {
                    let Ok(s) = state.lock() else {
                        return RpcResponse::internal_error(id, "Lock poisoned");
                    };
                    let events: Vec<_> = s.audit_log.all().iter()
                        .map(|e| serde_json::json!({
                            "timestamp": e.timestamp.to_rfc3339(),
                            "event_type": format!("{:?}", e.event_type),
                            "severity": format!("{:?}", e.severity),
                            "description": e.description,
                        }))
                        .collect();
                    RpcResponse::success(id, serde_json::json!(events))
                }
                // Stubs for methods that require UI-layer support
                Method::BrowserOpen | Method::BrowserNavigate | Method::BrowserEvalJs
                | Method::BrowserGetAccessibilityTree | Method::BrowserClickElement
                | Method::BrowserTypeText | Method::BrowserScreenshot
                | Method::TerminalScreenshot => {
                    RpcResponse::internal_error(id, "Method requires UI layer (not available via bridge IPC)")
                }
                // Everything else returns a basic success/not-implemented
                _ => {
                    RpcResponse::internal_error(id, &format!("Method {:?} not yet implemented in bridge IPC", method))
                }
            }
        })
    })
}
