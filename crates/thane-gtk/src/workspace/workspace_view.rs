use gtk4::prelude::*;

/// Container widget for a workspace's content area.
///
/// In Phase 1, this holds a single terminal. In later phases,
/// it will contain the SplitContainer for pane management.
pub struct WorkspaceView {
    container: gtk4::Box,
    stack: gtk4::Stack,
}

impl Default for WorkspaceView {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceView {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        let stack = gtk4::Stack::new();
        stack.set_transition_type(gtk4::StackTransitionType::Crossfade);
        stack.set_transition_duration(150);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        container.append(&stack);

        Self { container, stack }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Add a child widget (terminal or split container) to the stack.
    pub fn add_page(&self, name: &str, title: &str, widget: &impl IsA<gtk4::Widget>) {
        self.stack.add_titled(widget, Some(name), title);
    }

    /// Switch to a named page.
    pub fn set_visible_page(&self, name: &str) {
        self.stack.set_visible_child_name(name);
    }

    /// Get the stack (for direct manipulation).
    pub fn stack(&self) -> &gtk4::Stack {
        &self.stack
    }
}
