use std::cell::Cell;
use std::rc::Rc;

use thane_core::panel::PanelId;
use gtk4::prelude::*;
use vte4::prelude::*;

use crate::traits::{TerminalEngine, TerminalSurface};

/// VTE4-based terminal engine for Linux.
pub struct VteEngine {
    /// Default shell to use (falls back to $SHELL or /bin/bash).
    default_shell: String,
    /// Font description string (Pango format).
    font_desc: Option<String>,
    /// Scrollback lines limit (-1 for unlimited).
    scrollback_lines: i64,
}

impl VteEngine {
    pub fn new() -> Self {
        let default_shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        Self {
            default_shell,
            font_desc: None,
            scrollback_lines: 10000,
        }
    }

    pub fn set_font(&mut self, font_desc: impl Into<String>) {
        self.font_desc = Some(font_desc.into());
    }

    pub fn set_scrollback_lines(&mut self, lines: i64) {
        self.scrollback_lines = lines;
    }
}

impl Default for VteEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl VteEngine {
    /// Internal helper: configure a VTE terminal widget and spawn a shell.
    ///
    /// `child_setup` is called in the forked child before exec — this is where
    /// Landlock sandbox rules are applied.
    fn spawn_surface(
        &self,
        panel_id: PanelId,
        cwd: &str,
        shell: Option<&str>,
        extra_env: &[(&str, &str)],
        child_setup: impl Fn() + 'static,
    ) -> VteSurface {
        let terminal = vte4::Terminal::new();

        // Configure terminal
        terminal.set_scrollback_lines(self.scrollback_lines);
        terminal.set_scroll_on_output(false);
        terminal.set_scroll_on_keystroke(true);
        terminal.set_allow_hyperlink(true);

        // Selection highlight: muted blue-grey matching the app's theme.
        let sel_bg = gtk4::gdk::RGBA::new(0.25, 0.30, 0.45, 0.55);
        terminal.set_color_highlight(Some(&sel_bg));
        let sel_fg = gtk4::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0);
        terminal.set_color_highlight_foreground(Some(&sel_fg));

        // Set font if configured
        if let Some(ref font_desc) = self.font_desc {
            let font = gtk4::pango::FontDescription::from_string(font_desc);
            terminal.set_font(Some(&font));
        }

        // Spawn shell
        let shell = shell.unwrap_or(&self.default_shell);
        let shell_args = [shell];

        // Build environment: inherit parent env + extra vars.
        // When envv is empty, VTE inherits parent env. When non-empty, it
        // replaces the env entirely. So we must collect the full env.
        let envv: Vec<String> = if extra_env.is_empty() {
            Vec::new()
        } else {
            let mut env: Vec<String> = std::env::vars()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            for (k, v) in extra_env {
                env.push(format!("{k}={v}"));
            }
            env
        };
        let envv_refs: Vec<&str> = envv.iter().map(|s| s.as_str()).collect();

        let child_pid = Rc::new(Cell::new(None::<i32>));
        let child_pid_clone = child_pid.clone();

        terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(cwd),
            &shell_args,
            &envv_refs,
            glib::SpawnFlags::DEFAULT,
            child_setup,
            -1,                          // timeout (-1 = default)
            gtk4::gio::Cancellable::NONE,
            move |result| {
                match result {
                    Ok(pid) => {
                        tracing::debug!("Terminal spawned with PID: {pid:?}");
                        // glib::Pid is platform-specific. On Unix it's a c_int (i32).
                        child_pid_clone.set(Some(pid.0));
                    }
                    Err(e) => tracing::error!("Failed to spawn terminal: {e}"),
                }
            },
        );

        VteSurface {
            terminal,
            panel_id,
            child_pid,
        }
    }
}

impl TerminalEngine for VteEngine {
    type Surface = VteSurface;

    fn create_surface(
        &self,
        panel_id: PanelId,
        cwd: &str,
        shell: Option<&str>,
        extra_env: &[(&str, &str)],
    ) -> VteSurface {
        self.spawn_surface(panel_id, cwd, shell, extra_env, || {})
    }

    fn create_sandboxed_surface(
        &self,
        panel_id: PanelId,
        cwd: &str,
        shell: Option<&str>,
        extra_env: &[(&str, &str)],
        sandbox: &thane_core::sandbox::SandboxPolicy,
    ) -> VteSurface {
        // Merge sandbox environment variables into extra_env.
        let sandbox_env = sandbox.env_vars();
        let mut all_env: Vec<(&str, &str)> = extra_env.to_vec();
        let sandbox_refs: Vec<(&str, &str)> = sandbox_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        all_env.extend(sandbox_refs);

        // Serialize the policy for the child_setup closure.
        // We need to clone the data since child_setup must be 'static.
        let policy_json = serde_json::to_vec(sandbox).unwrap_or_default();
        let enabled = sandbox.enabled;

        self.spawn_surface(panel_id, cwd, shell, &all_env, move || {
            if !enabled || policy_json.is_empty() {
                return;
            }

            // Deserialize the policy in the child process.
            let policy: thane_core::sandbox::SandboxPolicy = match serde_json::from_slice(&policy_json) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("thane: failed to deserialize sandbox policy: {e}");
                    return;
                }
            };

            // Apply mount namespace isolation (hide denied paths entirely).
            // Must be done BEFORE Landlock so the mount tree is set up first.
            if let Err(e) = thane_platform::try_apply_mount_namespace(&policy) {
                eprintln!("thane: failed to apply mount namespace: {e}");
                // Non-fatal — Landlock still blocks access.
            }

            // Apply Landlock rules. This is in the forked child before exec,
            // so it only affects this shell and its descendants.
            if let Err(e) = thane_platform::apply_sandbox(&policy) {
                eprintln!("thane: failed to apply Landlock sandbox: {e}");
                // In permissive mode, continue anyway.
                if policy.enforcement != thane_core::sandbox::EnforcementLevel::Permissive {
                    // In enforcing/strict mode, abort the child.
                    std::process::exit(1);
                }
            }

            // Apply resource limits (RLIMIT_NOFILE, RLIMIT_FSIZE, RLIMIT_CPU).
            if let Err(e) = thane_platform::apply_resource_limits(&policy) {
                eprintln!("thane: failed to apply resource limits: {e}");
                if policy.enforcement != thane_core::sandbox::EnforcementLevel::Permissive {
                    std::process::exit(1);
                }
            }

            // Apply seccomp-bpf filter (Strict mode only — blocks dangerous syscalls).
            if let Err(e) = thane_platform::apply_seccomp(&policy) {
                eprintln!("thane: failed to apply seccomp filter: {e}");
                if policy.enforcement != thane_core::sandbox::EnforcementLevel::Permissive {
                    std::process::exit(1);
                }
            }
        })
    }
}

/// A VTE4 terminal surface wrapping a vte4::Terminal widget.
pub struct VteSurface {
    terminal: vte4::Terminal,
    panel_id: PanelId,
    child_pid: Rc<Cell<Option<i32>>>,
}

impl TerminalSurface for VteSurface {
    fn panel_id(&self) -> PanelId {
        self.panel_id
    }

    fn feed(&self, data: &[u8]) {
        self.terminal.feed(data);
    }

    fn feed_child(&self, text: &str) {
        self.terminal.feed_child(text.as_bytes());
    }

    fn cwd(&self) -> Option<String> {
        // VTE tracks CWD via OSC 7 if shell integration is set up.
        // We can also try reading /proc/<pid>/cwd.
        self.child_pid().and_then(|pid| {
            let cwd_link = format!("/proc/{pid}/cwd");
            std::fs::read_link(&cwd_link)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        })
    }

    fn title(&self) -> Option<String> {
        self.terminal.window_title().map(|s| s.to_string())
    }

    fn child_pid(&self) -> Option<u32> {
        self.child_pid.get().map(|pid| pid as u32)
    }

    fn copy_selection(&self) {
        self.terminal
            .copy_clipboard_format(vte4::Format::Text);
    }

    fn paste_clipboard(&self) {
        self.terminal.paste_clipboard();
    }

    fn get_text(&self) -> String {
        // Use text_format to get all visible text (requires VTE >= 0.76).
        self.terminal
            .text_format(vte4::Format::Text)
            .map(|s: glib::GString| s.to_string())
            .unwrap_or_default()
    }

    fn has_selection(&self) -> bool {
        self.terminal.has_selection()
    }

    fn scroll_up(&self, lines: u32) {
        // VTE implements GtkScrollable. Access vadjustment via the trait.
        let scrollable: &gtk4::Scrollable = self.terminal.upcast_ref();
        if let Some(adj) = scrollable.vadjustment() {
            let new_val = adj.value() - (lines as f64);
            adj.set_value(new_val.max(adj.lower()));
        }
    }

    fn scroll_down(&self, lines: u32) {
        let scrollable: &gtk4::Scrollable = self.terminal.upcast_ref();
        if let Some(adj) = scrollable.vadjustment() {
            let new_val = adj.value() + (lines as f64);
            adj.set_value(new_val.min(adj.upper() - adj.page_size()));
        }
    }

    fn grab_focus(&self) {
        self.terminal.grab_focus();
    }
}

impl VteSurface {
    /// Get the underlying GTK widget (for embedding in the UI).
    pub fn widget(&self) -> &gtk4::Widget {
        self.terminal.upcast_ref()
    }

    /// Get a reference to the underlying VTE terminal widget.
    pub fn vte_terminal(&self) -> &vte4::Terminal {
        &self.terminal
    }

    /// Set a search regex pattern for find-in-terminal.
    pub fn search_set_pattern(&self, pattern: &str) -> bool {
        if pattern.is_empty() {
            self.terminal.search_set_regex(None::<&vte4::Regex>, 0);
            return true;
        }

        match vte4::Regex::for_search(pattern, 0) {
            Ok(regex) => {
                self.terminal.search_set_regex(Some(&regex), 0);
                self.terminal.search_set_wrap_around(true);
                true
            }
            Err(_) => false,
        }
    }

    /// Find next match.
    pub fn search_find_next(&self) -> bool {
        self.terminal.search_find_next()
    }

    /// Find previous match.
    pub fn search_find_previous(&self) -> bool {
        self.terminal.search_find_previous()
    }

    /// Clear search highlighting.
    pub fn search_clear(&self) {
        self.terminal.search_set_regex(None::<&vte4::Regex>, 0);
    }

    /// Connect to the terminal's `child-exited` signal.
    pub fn connect_child_exited<F: Fn(i32) + 'static>(&self, f: F) {
        self.terminal.connect_child_exited(move |_term, status| {
            f(status);
        });
    }

    /// Connect to the terminal's `window-title-changed` signal.
    pub fn connect_title_changed<F: Fn(String) + 'static>(&self, f: F) {
        self.terminal.connect_window_title_changed(move |term| {
            let title = term
                .window_title()
                .map(|s| s.to_string())
                .unwrap_or_default();
            f(title);
        });
    }

    /// Connect to the terminal's `commit` signal for raw output interception.
    /// This receives all text committed to the terminal, including escape sequences.
    /// Callback receives the raw text data.
    pub fn connect_commit<F: Fn(&str) + 'static>(&self, f: F) {
        self.terminal.connect_commit(move |_term, text, _size| {
            f(text);
        });
    }

    /// Connect to the terminal's `current-directory-uri-changed` signal.
    /// Fires when the shell reports CWD via OSC 7.
    pub fn connect_cwd_changed<F: Fn(Option<String>) + 'static>(&self, f: F) {
        self.terminal
            .connect_current_directory_uri_changed(move |term| {
                let uri = term.current_directory_uri().map(|s| {
                    // Parse file:// URI to plain path.
                    let s = s.to_string();
                    if let Some(rest) = s.strip_prefix("file://")
                        && let Some(slash_pos) = rest.find('/')
                    {
                        return rest[slash_pos..].to_string();
                    }
                    s
                });
                f(uri);
            });
    }

    /// Connect to the terminal's `bell` signal.
    pub fn connect_bell<F: Fn() + 'static>(&self, f: F) {
        self.terminal.connect_bell(move |_term| {
            f();
        });
    }

    /// Connect a click handler for OSC 8 hyperlinks in the terminal.
    /// The callback receives (url, shift_held). Normal click → embedded browser;
    /// Shift+click → system browser.
    pub fn connect_hyperlink_clicked<F: Fn(&str, bool) + 'static>(&self, f: F) {
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1); // Left mouse button only.

        let terminal = self.terminal.clone();
        gesture.connect_released(move |gesture, _n_press, x, y| {
            // Check if there's a hyperlink at the click position.
            if let Some(uri) = terminal.check_hyperlink_at(x, y) {
                let uri_str = uri.as_str();
                if !uri_str.is_empty() {
                    // Determine if Shift was held.
                    let shift_held = gesture
                        .current_event_state()
                        .contains(gdk4::ModifierType::SHIFT_MASK);
                    f(uri_str, shift_held);
                    // Claim the gesture so VTE doesn't also handle it.
                    gesture.set_state(gtk4::EventSequenceState::Claimed);
                }
            }
        });

        self.terminal.add_controller(gesture);
    }
}
