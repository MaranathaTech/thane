use thane_core::panel::PanelId;
use thane_core::sandbox::SandboxPolicy;

/// Abstraction over a terminal emulator engine.
///
/// This trait allows swapping between VTE (Linux), ConPTY (Windows),
/// SwiftTerm (macOS), or other backends without changing the rest of the codebase.
pub trait TerminalEngine {
    type Surface: TerminalSurface;

    /// Create a new terminal surface.
    ///
    /// `extra_env` provides additional environment variables to set in the
    /// spawned shell process (e.g. `THANE_WORKSPACE_ID`).
    fn create_surface(
        &self,
        panel_id: PanelId,
        cwd: &str,
        shell: Option<&str>,
        extra_env: &[(&str, &str)],
    ) -> Self::Surface;

    /// Create a new sandboxed terminal surface.
    ///
    /// Like `create_surface`, but applies the given sandbox policy to the
    /// child process using Landlock LSM (if supported).
    fn create_sandboxed_surface(
        &self,
        panel_id: PanelId,
        cwd: &str,
        shell: Option<&str>,
        extra_env: &[(&str, &str)],
        sandbox: &SandboxPolicy,
    ) -> Self::Surface {
        // Default: merge sandbox env vars and create a regular surface.
        let sandbox_env = sandbox.env_vars();
        let mut all_env: Vec<(&str, &str)> = extra_env.to_vec();
        let sandbox_refs: Vec<(&str, &str)> = sandbox_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        all_env.extend(sandbox_refs);
        self.create_surface(panel_id, cwd, shell, &all_env)
    }
}

/// A single terminal emulator surface.
///
/// Platform-agnostic interface for terminal operations.
/// Widget access (for embedding in the UI) is provided by concrete backend
/// types (e.g. `VteSurface::widget()`) rather than through this trait,
/// since widget types differ across platforms.
pub trait TerminalSurface {
    /// Get the panel ID this surface belongs to.
    fn panel_id(&self) -> PanelId;

    /// Feed raw bytes to the terminal display (from a PTY or socket).
    fn feed(&self, data: &[u8]);

    /// Send text to the terminal's child process (as if the user typed it).
    fn feed_child(&self, text: &str);

    /// Get the current working directory of the terminal's child process.
    fn cwd(&self) -> Option<String>;

    /// Get the current title (from terminal escape sequences).
    fn title(&self) -> Option<String>;

    /// Get the terminal's child process PID.
    fn child_pid(&self) -> Option<u32>;

    /// Copy selected text to clipboard.
    fn copy_selection(&self);

    /// Paste from clipboard.
    fn paste_clipboard(&self);

    /// Get all terminal text content (for session persistence).
    fn get_text(&self) -> String;

    /// Check if the terminal has a selection.
    fn has_selection(&self) -> bool;

    /// Scroll up by the given number of lines.
    fn scroll_up(&self, lines: u32);

    /// Scroll down by the given number of lines.
    fn scroll_down(&self, lines: u32);

    /// Grab focus.
    fn grab_focus(&self);
}
