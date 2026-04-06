use std::collections::HashMap;

use uuid::Uuid;

use crate::audit::SensitiveOpAction;
use crate::error::CoreError;
use crate::notification::NotificationStore;
use crate::pane::{Orientation, PaneId, SplitTree};
use crate::panel::{PanelId, PanelInfo};
use crate::sandbox::SandboxPolicy;
use crate::session::WorkspaceSnapshot;
use crate::sidebar::SidebarMetadata;

/// A workspace represents a project context: a set of panes (terminals/browsers)
/// arranged in a split tree, with associated metadata.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: Uuid,
    pub title: String,
    pub cwd: String,
    pub tag: Option<String>,
    pub split_tree: SplitTree,
    pub panels: HashMap<PanelId, PanelInfo>,
    pub focused_pane_id: PaneId,
    pub sidebar: SidebarMetadata,
    pub notifications: NotificationStore,
    pub sandbox_policy: SandboxPolicy,
    /// What to do when sensitive file access or PII is detected in terminal output.
    pub sensitive_op_action: SensitiveOpAction,
}

impl Workspace {
    /// Create a new workspace with a single terminal panel.
    pub fn new(title: impl Into<String>, cwd: impl Into<String>) -> Self {
        let cwd = cwd.into();
        let panel = PanelInfo::new_terminal("shell", &cwd);
        let panel_id = panel.id;
        let split_tree = SplitTree::new_leaf(panel_id);
        let focused_pane_id = split_tree.pane_ids()[0];

        let mut panels = HashMap::new();
        panels.insert(panel_id, panel);

        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            cwd,
            tag: None,
            split_tree,
            panels,
            focused_pane_id,
            sidebar: SidebarMetadata::default(),
            notifications: NotificationStore::new(1000),
            sandbox_policy: SandboxPolicy::default(),
            sensitive_op_action: SensitiveOpAction::default(),
        }
    }

    /// Restore a workspace from a session snapshot, preserving the full split tree
    /// and all panel info so that splits, tabs, and divider positions are restored.
    pub fn restore_from_snapshot(snap: &WorkspaceSnapshot) -> Self {
        let panels: HashMap<PanelId, PanelInfo> = snap
            .panels
            .iter()
            .map(|ps| (ps.info.id, ps.info.clone()))
            .collect();

        let effective_focus = snap
            .focused_pane_id
            .filter(|fp| snap.split_tree.find_pane(*fp).is_some())
            .unwrap_or_else(|| snap.split_tree.pane_ids()[0]);

        Self {
            id: snap.id,
            title: snap.title.clone(),
            cwd: snap.cwd.clone(),
            tag: snap.tag.clone(),
            split_tree: snap.split_tree.clone(),
            panels,
            focused_pane_id: effective_focus,
            sidebar: SidebarMetadata::default(),
            notifications: NotificationStore::new(1000),
            sandbox_policy: snap.sandbox_policy.clone(),
            sensitive_op_action: SensitiveOpAction::default(),
        }
    }

    /// Add a new terminal panel and split the focused pane.
    pub fn split_terminal(
        &mut self,
        orientation: Orientation,
    ) -> Result<(PaneId, PanelId), CoreError> {
        let panel = PanelInfo::new_terminal("shell", &self.cwd);
        let panel_id = panel.id;
        self.panels.insert(panel_id, panel);

        let new_pane_id =
            self.split_tree
                .split(self.focused_pane_id, panel_id, orientation)?;
        Ok((new_pane_id, panel_id))
    }

    /// Add a new browser panel and split the focused pane.
    pub fn split_browser(
        &mut self,
        url: impl Into<String>,
        orientation: Orientation,
    ) -> Result<(PaneId, PanelId), CoreError> {
        let url = url.into();
        let panel = PanelInfo::new_browser("Browser", &url);
        let panel_id = panel.id;
        self.panels.insert(panel_id, panel);

        let new_pane_id =
            self.split_tree
                .split(self.focused_pane_id, panel_id, orientation)?;
        Ok((new_pane_id, panel_id))
    }

    /// Close a pane and remove its panels.
    pub fn close_pane(&mut self, pane_id: PaneId) -> Result<(), CoreError> {
        // Collect panel IDs from the pane before closing.
        let panel_ids = if let Some(SplitTree::Leaf { panel_ids, .. }) =
            self.split_tree.find_pane(pane_id)
        {
            panel_ids.clone()
        } else {
            return Err(CoreError::PaneNotFound(pane_id));
        };

        self.split_tree.close_pane(pane_id)?;

        // Remove associated panels.
        for id in &panel_ids {
            self.panels.remove(id);
        }

        // Update focus if the focused pane was closed.
        if self.focused_pane_id == pane_id {
            self.focused_pane_id = self.split_tree.pane_ids()[0];
        }

        Ok(())
    }

    /// Focus the next pane in order.
    pub fn focus_next_pane(&mut self) {
        if let Some(next) = self.split_tree.next_pane(self.focused_pane_id) {
            self.focused_pane_id = next;
        }
    }

    /// Focus the previous pane in order.
    pub fn focus_prev_pane(&mut self) {
        if let Some(prev) = self.split_tree.prev_pane(self.focused_pane_id) {
            self.focused_pane_id = prev;
        }
    }

    /// Get the currently selected panel in the focused pane.
    pub fn focused_panel(&self) -> Option<&PanelInfo> {
        if let Some(SplitTree::Leaf {
            selected_panel, ..
        }) = self.split_tree.find_pane(self.focused_pane_id)
        {
            self.panels.get(selected_panel)
        } else {
            None
        }
    }

    /// Get the number of panes in this workspace.
    pub fn pane_count(&self) -> usize {
        self.split_tree.pane_count()
    }

    /// Add a browser panel as a new tab in the focused pane.
    pub fn add_browser_to_focused_pane(
        &mut self,
        url: impl Into<String>,
    ) -> Result<PanelId, CoreError> {
        let url = url.into();
        let panel = PanelInfo::new_browser("Browser", &url);
        let panel_id = panel.id;
        self.panels.insert(panel_id, panel);
        if !self
            .split_tree
            .add_panel_to_pane(self.focused_pane_id, panel_id)
        {
            self.panels.remove(&panel_id);
            return Err(CoreError::PaneNotFound(self.focused_pane_id));
        }
        Ok(panel_id)
    }

    /// Close a panel from a pane. If the pane becomes empty and is not the last pane,
    /// closes the pane too. Returns true if the pane was also closed.
    pub fn close_panel(
        &mut self,
        pane_id: crate::pane::PaneId,
        panel_id: PanelId,
    ) -> Result<bool, CoreError> {
        let pane_empty = self
            .split_tree
            .remove_panel_from_pane(pane_id, panel_id)
            .ok_or(CoreError::PanelNotFound(panel_id))?;

        self.panels.remove(&panel_id);

        if pane_empty {
            if self.split_tree.pane_count() <= 1 {
                // Last pane — don't close it, just leave it empty.
                // The caller should handle this (e.g., close workspace).
                return Ok(true);
            }
            self.split_tree.close_pane(pane_id)?;
            if self.focused_pane_id == pane_id {
                self.focused_pane_id = self.split_tree.pane_ids()[0];
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Select a panel in the focused pane.
    pub fn select_panel(&mut self, panel_id: PanelId) -> bool {
        self.split_tree
            .select_panel_in_pane(self.focused_pane_id, panel_id)
    }

    /// Reorder a panel within the focused pane.
    pub fn reorder_panel(&mut self, panel_id: PanelId, new_index: usize) -> bool {
        self.split_tree.reorder_panel_in_pane(self.focused_pane_id, panel_id, new_index)
    }

    /// Swap the positions of two panels within a pane.
    /// The pane is found by looking up `panel_a`; both panels must be in the same pane.
    /// Returns `true` if the swap succeeded.
    pub fn swap_panels_in_pane(&mut self, panel_a: PanelId, panel_b: PanelId) -> bool {
        let pane_id = match self.split_tree.pane_for_panel(panel_a) {
            Some(id) => id,
            None => return false,
        };
        self.split_tree.swap_panels_in_pane(pane_id, panel_a, panel_b)
    }

    /// Cycle to the next panel in the focused pane, returning its ID.
    pub fn next_panel(&mut self) -> Option<PanelId> {
        let next = self.split_tree.next_panel_in_pane(self.focused_pane_id)?;
        self.split_tree
            .select_panel_in_pane(self.focused_pane_id, next);
        Some(next)
    }

    /// Cycle to the previous panel in the focused pane, returning its ID.
    pub fn prev_panel(&mut self) -> Option<PanelId> {
        let prev = self.split_tree.prev_panel_in_pane(self.focused_pane_id)?;
        self.split_tree
            .select_panel_in_pane(self.focused_pane_id, prev);
        Some(prev)
    }

    /// Find which pane contains a given panel ID.
    pub fn pane_for_panel(&self, panel_id: PanelId) -> Option<crate::pane::PaneId> {
        self.split_tree.pane_for_panel(panel_id)
    }
}

/// Manages all workspaces in the application.
#[derive(Debug)]
pub struct WorkspaceManager {
    workspaces: Vec<Workspace>,
    active_index: usize,
}

impl WorkspaceManager {
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            active_index: 0,
        }
    }

    /// Create a new workspace and make it active.
    pub fn create(&mut self, title: impl Into<String>, cwd: impl Into<String>) -> &Workspace {
        let workspace = Workspace::new(title, cwd);
        self.workspaces.push(workspace);
        self.active_index = self.workspaces.len() - 1;
        &self.workspaces[self.active_index]
    }

    /// Add an already-created workspace (used for session restore).
    pub fn add(&mut self, workspace: Workspace) -> &Workspace {
        self.workspaces.push(workspace);
        self.active_index = self.workspaces.len() - 1;
        &self.workspaces[self.active_index]
    }

    /// Get the active workspace.
    pub fn active(&self) -> Option<&Workspace> {
        self.workspaces.get(self.active_index)
    }

    /// Get the active workspace mutably.
    pub fn active_mut(&mut self) -> Option<&mut Workspace> {
        self.workspaces.get_mut(self.active_index)
    }

    /// Select workspace by index (0-based).
    pub fn select(&mut self, index: usize) -> bool {
        if index < self.workspaces.len() {
            self.active_index = index;
            true
        } else {
            false
        }
    }

    /// Select workspace by ID.
    pub fn select_by_id(&mut self, id: Uuid) -> bool {
        if let Some(index) = self.workspaces.iter().position(|w| w.id == id) {
            self.active_index = index;
            true
        } else {
            false
        }
    }

    /// Select next workspace (cycling).
    pub fn select_next(&mut self) {
        if !self.workspaces.is_empty() {
            self.active_index = (self.active_index + 1) % self.workspaces.len();
        }
    }

    /// Select previous workspace (cycling).
    pub fn select_prev(&mut self) {
        if !self.workspaces.is_empty() {
            self.active_index = if self.active_index == 0 {
                self.workspaces.len() - 1
            } else {
                self.active_index - 1
            };
        }
    }

    /// Close the active workspace. Returns the removed workspace.
    pub fn close_active(&mut self) -> Option<Workspace> {
        if self.workspaces.is_empty() {
            return None;
        }
        let removed = self.workspaces.remove(self.active_index);
        if self.active_index >= self.workspaces.len() && !self.workspaces.is_empty() {
            self.active_index = self.workspaces.len() - 1;
        }
        Some(removed)
    }

    /// Close a workspace by ID. Returns the removed workspace.
    pub fn close_by_id(&mut self, id: Uuid) -> Option<Workspace> {
        if let Some(index) = self.workspaces.iter().position(|w| w.id == id) {
            let removed = self.workspaces.remove(index);
            if self.active_index >= self.workspaces.len() && !self.workspaces.is_empty() {
                self.active_index = self.workspaces.len() - 1;
            }
            Some(removed)
        } else {
            None
        }
    }

    /// Rename the active workspace.
    pub fn rename_active(&mut self, title: impl Into<String>) -> bool {
        if let Some(ws) = self.workspaces.get_mut(self.active_index) {
            ws.title = title.into();
            true
        } else {
            false
        }
    }

    /// Set a tag on the active workspace.
    pub fn set_active_tag(&mut self, tag: Option<String>) -> bool {
        if let Some(ws) = self.workspaces.get_mut(self.active_index) {
            ws.tag = tag;
            true
        } else {
            false
        }
    }

    /// Get a workspace by ID.
    pub fn get(&self, id: Uuid) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    /// Get a workspace by ID mutably.
    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    /// List all workspaces.
    pub fn list(&self) -> &[Workspace] {
        &self.workspaces
    }

    /// Get the active workspace index.
    pub fn active_index(&self) -> usize {
        self.active_index
    }

    /// Get workspace count.
    pub fn count(&self) -> usize {
        self.workspaces.len()
    }

    /// Check if there are any workspaces.
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_creation() {
        let ws = Workspace::new("Test", "/home/user");
        assert_eq!(ws.title, "Test");
        assert_eq!(ws.cwd, "/home/user");
        assert_eq!(ws.pane_count(), 1);
        assert_eq!(ws.panels.len(), 1);
    }

    #[test]
    fn test_workspace_split() {
        let mut ws = Workspace::new("Test", "/home/user");
        let (_new_pane, _panel_id) = ws.split_terminal(Orientation::Horizontal).unwrap();
        assert_eq!(ws.pane_count(), 2);
        assert_eq!(ws.panels.len(), 2);
    }

    #[test]
    fn test_workspace_close_pane() {
        let mut ws = Workspace::new("Test", "/home/user");
        let (new_pane, _) = ws.split_terminal(Orientation::Horizontal).unwrap();
        ws.close_pane(new_pane).unwrap();
        assert_eq!(ws.pane_count(), 1);
        assert_eq!(ws.panels.len(), 1);
    }

    #[test]
    fn test_workspace_manager() {
        let mut mgr = WorkspaceManager::new();
        assert!(mgr.is_empty());

        mgr.create("Workspace 1", "/home/user/project1");
        mgr.create("Workspace 2", "/home/user/project2");
        assert_eq!(mgr.count(), 2);
        assert_eq!(mgr.active_index(), 1);

        mgr.select(0);
        assert_eq!(mgr.active().unwrap().title, "Workspace 1");

        mgr.select_next();
        assert_eq!(mgr.active().unwrap().title, "Workspace 2");

        mgr.select_next(); // Should cycle
        assert_eq!(mgr.active().unwrap().title, "Workspace 1");
    }

    #[test]
    fn test_add_browser_to_focused_pane() {
        let mut ws = Workspace::new("Test", "/home");
        let original_panel_count = ws.panels.len();
        let panel_id = ws
            .add_browser_to_focused_pane("http://localhost:3000")
            .unwrap();
        assert_eq!(ws.panels.len(), original_panel_count + 1);
        assert_eq!(ws.pane_count(), 1); // Still one pane.
        assert!(ws.panels.get(&panel_id).is_some());
    }

    #[test]
    fn test_close_panel() {
        let mut ws = Workspace::new("Test", "/home");
        let original_panel_id = ws.focused_panel().unwrap().id;
        let pane_id = ws.focused_pane_id;

        let browser_id = ws
            .add_browser_to_focused_pane("http://localhost:3000")
            .unwrap();

        // Close the browser panel — pane should remain.
        let pane_closed = ws.close_panel(pane_id, browser_id).unwrap();
        assert!(!pane_closed);
        assert_eq!(ws.panels.len(), 1);
        assert_eq!(ws.focused_panel().unwrap().id, original_panel_id);
    }

    #[test]
    fn test_next_prev_panel() {
        let mut ws = Workspace::new("Test", "/home");
        let panel1 = ws.focused_panel().unwrap().id;
        let panel2 = ws
            .add_browser_to_focused_pane("http://localhost:3000")
            .unwrap();

        // Selected is panel2 (just added).
        assert_eq!(ws.next_panel(), Some(panel1));
        assert_eq!(ws.prev_panel(), Some(panel2));
    }

    #[test]
    fn test_pane_for_panel() {
        let ws = Workspace::new("Test", "/home");
        let panel1 = ws.focused_panel().unwrap().id;
        let pane_id = ws.focused_pane_id;

        assert_eq!(ws.pane_for_panel(panel1), Some(pane_id));
        assert_eq!(ws.pane_for_panel(Uuid::new_v4()), None);
    }

    #[test]
    fn test_workspace_manager_close() {
        let mut mgr = WorkspaceManager::new();
        mgr.create("WS1", "/tmp");
        mgr.create("WS2", "/tmp");
        mgr.create("WS3", "/tmp");

        mgr.select(1);
        let closed = mgr.close_active().unwrap();
        assert_eq!(closed.title, "WS2");
        assert_eq!(mgr.count(), 2);
        assert_eq!(mgr.active().unwrap().title, "WS3");
    }

    #[test]
    fn test_workspace_manager_close_by_id() {
        let mut mgr = WorkspaceManager::new();
        let ws1 = mgr.create("WS1", "/tmp").id;
        mgr.create("WS2", "/tmp");

        let closed = mgr.close_by_id(ws1);
        assert!(closed.is_some());
        assert_eq!(closed.unwrap().title, "WS1");
        assert_eq!(mgr.count(), 1);

        let missing = mgr.close_by_id(Uuid::new_v4());
        assert!(missing.is_none());
    }

    #[test]
    fn test_close_panel_removes_pane_when_empty() {
        let mut ws = Workspace::new("Test", "/home");
        // Split so we have two panes
        let (new_pane, new_panel_id) = ws.split_terminal(Orientation::Horizontal).unwrap();
        assert_eq!(ws.pane_count(), 2);

        // Close the only panel in the new pane — should remove the pane
        let pane_closed = ws.close_panel(new_pane, new_panel_id).unwrap();
        assert!(pane_closed);
        assert_eq!(ws.pane_count(), 1);
    }

    #[test]
    fn test_focus_next_prev_pane_cycling() {
        let mut ws = Workspace::new("Test", "/home");
        let original_pane = ws.focused_pane_id;
        let (new_pane, _) = ws.split_terminal(Orientation::Horizontal).unwrap();

        // Focus should still be on original pane
        assert_eq!(ws.focused_pane_id, original_pane);

        ws.focus_next_pane();
        assert_eq!(ws.focused_pane_id, new_pane);

        ws.focus_next_pane(); // Cycle back
        assert_eq!(ws.focused_pane_id, original_pane);

        ws.focus_prev_pane(); // Should go to new_pane
        assert_eq!(ws.focused_pane_id, new_pane);
    }

    #[test]
    fn test_select_by_id_nonexistent() {
        let mut mgr = WorkspaceManager::new();
        mgr.create("WS1", "/tmp");
        assert!(!mgr.select_by_id(Uuid::new_v4()));
        // Active index should remain unchanged
        assert_eq!(mgr.active_index(), 0);
    }

    #[test]
    fn test_rename_active() {
        let mut mgr = WorkspaceManager::new();
        mgr.create("Original", "/tmp");
        assert!(mgr.rename_active("Renamed"));
        assert_eq!(mgr.active().unwrap().title, "Renamed");
    }

    #[test]
    fn test_rename_active_empty_manager() {
        let mut mgr = WorkspaceManager::new();
        assert!(!mgr.rename_active("Nothing"));
    }

    #[test]
    fn test_set_active_tag() {
        let mut mgr = WorkspaceManager::new();
        mgr.create("WS1", "/tmp");
        assert!(mgr.set_active_tag(Some("rust".to_string())));
        assert_eq!(mgr.active().unwrap().tag.as_deref(), Some("rust"));
        assert!(mgr.set_active_tag(None));
        assert!(mgr.active().unwrap().tag.is_none());
    }

    #[test]
    fn test_close_active_last_workspace_adjusts_index() {
        let mut mgr = WorkspaceManager::new();
        mgr.create("WS1", "/tmp");
        mgr.create("WS2", "/tmp");
        mgr.create("WS3", "/tmp");

        // Select last workspace (index 2)
        mgr.select(2);
        assert_eq!(mgr.active().unwrap().title, "WS3");

        // Close it — index should adjust to the new last
        let closed = mgr.close_active().unwrap();
        assert_eq!(closed.title, "WS3");
        assert_eq!(mgr.active_index(), 1);
        assert_eq!(mgr.active().unwrap().title, "WS2");
    }

    #[test]
    fn test_select_prev_cycling() {
        let mut mgr = WorkspaceManager::new();
        mgr.create("WS1", "/tmp");
        mgr.create("WS2", "/tmp");

        mgr.select(0);
        mgr.select_prev(); // Should cycle to last
        assert_eq!(mgr.active().unwrap().title, "WS2");
    }

    #[test]
    fn test_swap_panels_in_pane() {
        let mut ws = Workspace::new("Test", "/home");
        let panel1 = ws.focused_panel().unwrap().id;
        let panel2 = ws
            .add_browser_to_focused_pane("http://localhost:3000")
            .unwrap();
        let panel3 = ws
            .add_browser_to_focused_pane("http://localhost:8080")
            .unwrap();

        // Order is [panel1, panel2, panel3]. Swap panel1 and panel3.
        assert!(ws.swap_panels_in_pane(panel1, panel3));

        // Verify new order: [panel3, panel2, panel1].
        let pane = ws.split_tree.find_pane(ws.focused_pane_id).unwrap();
        if let SplitTree::Leaf { panel_ids, .. } = pane {
            assert_eq!(panel_ids[0], panel3);
            assert_eq!(panel_ids[1], panel2);
            assert_eq!(panel_ids[2], panel1);
        } else {
            panic!("Expected leaf");
        }
    }

    #[test]
    fn test_swap_panels_in_pane_nonexistent() {
        let mut ws = Workspace::new("Test", "/home");
        let panel1 = ws.focused_panel().unwrap().id;
        let fake_id = Uuid::new_v4();

        // Swapping with a non-existent panel should fail.
        assert!(!ws.swap_panels_in_pane(panel1, fake_id));

        // Swapping two non-existent panels should fail.
        assert!(!ws.swap_panels_in_pane(Uuid::new_v4(), Uuid::new_v4()));
    }

    #[test]
    fn test_swap_panels_same_panel() {
        let mut ws = Workspace::new("Test", "/home");
        let panel1 = ws.focused_panel().unwrap().id;
        ws.add_browser_to_focused_pane("http://localhost:3000")
            .unwrap();

        // Swapping a panel with itself should succeed (no-op).
        assert!(ws.swap_panels_in_pane(panel1, panel1));
    }
}
