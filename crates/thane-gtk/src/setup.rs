use std::path::PathBuf;

use gtk4::prelude::*;

/// Dependency check result.
pub struct DepStatus {
    pub has_node: bool,
    pub has_claude: bool,
}

/// Check whether Node.js and Claude Code are available on this system.
pub fn check_dependencies() -> DepStatus {
    let has_node = std::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let has_claude = thane_core::queue_executor::which_claude().is_some();

    DepStatus {
        has_node,
        has_claude,
    }
}

/// Run every-launch startup checks: dependency verification, optional install, and CLAUDE.md injection.
pub fn run_startup_checks(window: &gtk4::ApplicationWindow) {
    let status = check_dependencies();

    if !status.has_node {
        show_missing_node_dialog(window);
    } else if !status.has_claude {
        show_install_claude_dialog(window);
    } else {
        // Both present — silently inject CLAUDE.md instructions if needed.
        inject_claude_md_if_needed();
    }

    // Always attempt to make thane-cli available on PATH (silent, best-effort).
    ensure_cli_available();
}

/// Show a dialog telling the user Node.js is required.
fn show_missing_node_dialog(window: &gtk4::ApplicationWindow) {
    let (dialog, vbox) = styled_setup_dialog(window, 480, 200);

    let icon = gtk4::Label::new(Some("\u{26a0}"));
    icon.set_css_classes(&["token-cost-large"]);
    vbox.append(&icon);

    let msg = gtk4::Label::new(Some(
        "Node.js is required but not installed.\n\n\
         Please install Node.js from https://nodejs.org\n\
         and restart thane.",
    ));
    msg.set_wrap(true);
    msg.set_justify(gtk4::Justification::Center);
    vbox.append(&msg);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);
    let ok_btn = gtk4::Button::with_label("OK");
    let dialog_ref = dialog.clone();
    ok_btn.connect_clicked(move |_| {
        dialog_ref.close();
    });
    btn_box.append(&ok_btn);
    vbox.append(&btn_box);

    dialog.present();
}

/// Show a dialog that attempts to install Claude Code via npm.
fn show_install_claude_dialog(window: &gtk4::ApplicationWindow) {
    let (dialog, vbox) = styled_setup_dialog(window, 480, 220);

    let spinner = gtk4::Spinner::new();
    spinner.start();
    vbox.append(&spinner);

    let status_label = gtk4::Label::new(Some("Installing Claude Code..."));
    status_label.set_wrap(true);
    status_label.set_justify(gtk4::Justification::Center);
    vbox.append(&status_label);

    let detail_label = gtk4::Label::new(Some("Running: npm install -g @anthropic-ai/claude-code"));
    detail_label.add_css_class("dim-label");
    detail_label.set_wrap(true);
    vbox.append(&detail_label);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    btn_box.set_margin_top(8);
    let ok_btn = gtk4::Button::with_label("OK");
    ok_btn.set_visible(false);
    btn_box.append(&ok_btn);
    vbox.append(&btn_box);

    dialog.present();

    // Spawn background thread for npm install — only Send data crosses thread boundary.
    // The install result (a simple enum with String) is sent back via glib::idle_add_local_once.
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();

    std::thread::spawn(move || {
        let result = run_npm_install();
        let _ = tx.send(result);
    });

    // Poll for the result from the GTK main thread.
    let spinner_ref = spinner;
    let status_ref = status_label;
    let detail_ref = detail_label;
    let ok_ref = ok_btn;
    let dialog_ref = dialog;

    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        match rx.try_recv() {
            Ok(result) => {
                spinner_ref.stop();
                spinner_ref.set_visible(false);

                match result {
                    Ok(()) => {
                        status_ref.set_text("Claude Code installed successfully!");
                        detail_ref.set_visible(false);

                        // Inject CLAUDE.md instructions now that Claude is available.
                        inject_claude_md_if_needed();

                        // Auto-close after 2 seconds.
                        let d = dialog_ref.clone();
                        glib::timeout_add_seconds_local_once(2, move || {
                            d.close();
                        });
                    }
                    Err(err) => {
                        status_ref.set_text("Failed to install Claude Code.");
                        detail_ref.set_text(&format!(
                            "Please run manually:\n  npm install -g @anthropic-ai/claude-code\n\nError: {err}"
                        ));
                        ok_ref.set_visible(true);
                        let d = dialog_ref.clone();
                        ok_ref.connect_clicked(move |_| {
                            d.close();
                        });
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Still waiting — keep polling.
                glib::ControlFlow::Continue
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Thread panicked or dropped sender — treat as failure.
                spinner_ref.stop();
                spinner_ref.set_visible(false);
                status_ref.set_text("Installation failed unexpectedly.");
                ok_ref.set_visible(true);
                let d = dialog_ref.clone();
                ok_ref.connect_clicked(move |_| {
                    d.close();
                });
                glib::ControlFlow::Break
            }
        }
    });
}

/// Run `npm install -g @anthropic-ai/claude-code` and return Ok(()) on success or Err(message).
fn run_npm_install() -> Result<(), String> {
    let output = std::process::Command::new("npm")
        .args(["install", "-g", "@anthropic-ai/claude-code"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let msg = if !stderr.is_empty() {
                stderr.to_string()
            } else {
                stdout.to_string()
            };
            Err(msg)
        }
        Err(e) => Err(format!("Failed to run npm: {e}")),
    }
}

/// Ensure `thane-cli` is reachable on PATH by symlinking it into `~/.local/bin/`.
///
/// If `thane-cli` is already on PATH and points to a valid binary, this is a no-op.
/// Otherwise, locates `thane-cli` next to the running `thane` executable and creates
/// (or replaces a stale) symlink at `~/.local/bin/thane-cli`.
fn ensure_cli_available() {
    // 1. Already reachable?
    if which_thane_cli() {
        tracing::debug!("thane-cli already on PATH");
        return;
    }

    // 2. Find thane-cli next to our own executable.
    let cli_binary = match find_cli_next_to_exe() {
        Some(p) => p,
        None => {
            tracing::warn!(
                "thane-cli binary not found next to the running executable; \
                 users will need to install it manually"
            );
            return;
        }
    };

    // 3. Ensure ~/.local/bin exists.
    let local_bin = match dirs::home_dir() {
        Some(home) => home.join(".local").join("bin"),
        None => {
            tracing::warn!("Could not determine home directory; skipping thane-cli symlink");
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&local_bin) {
        tracing::warn!("Failed to create {}: {e}", local_bin.display());
        return;
    }

    let link_path = local_bin.join("thane-cli");

    // 4. If a symlink/file already exists, check if it points to the right place.
    if link_path.symlink_metadata().is_ok() {
        match std::fs::read_link(&link_path) {
            Ok(target) if target == cli_binary => {
                // Symlink exists and is correct — nothing to do.
                tracing::debug!("thane-cli symlink already correct");
                return;
            }
            _ => {
                // Stale symlink or regular file — remove it.
                if let Err(e) = std::fs::remove_file(&link_path) {
                    tracing::warn!("Failed to remove stale {}: {e}", link_path.display());
                    return;
                }
            }
        }
    }

    // 5. Create the symlink.
    #[cfg(unix)]
    {
        match std::os::unix::fs::symlink(&cli_binary, &link_path) {
            Ok(()) => {
                tracing::info!(
                    "Symlinked thane-cli: {} → {}",
                    link_path.display(),
                    cli_binary.display()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to symlink {} → {}: {e}",
                    link_path.display(),
                    cli_binary.display()
                );
            }
        }
    }
}

/// Check if `thane-cli` is reachable via PATH.
fn which_thane_cli() -> bool {
    std::process::Command::new("thane-cli")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Locate `thane-cli` binary in the same directory as the running `thane` executable.
fn find_cli_next_to_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("thane-cli");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

/// Silently inject thane instructions into `~/.claude/CLAUDE.md` if not already present.
fn inject_claude_md_if_needed() {
    if !thane_platform::claude_md::has_thane_instructions() {
        match thane_platform::claude_md::inject_thane_instructions() {
            Ok(true) => tracing::info!("Injected thane instructions into ~/.claude/CLAUDE.md"),
            Ok(false) => {}
            Err(e) => tracing::error!("Failed to inject CLAUDE.md instructions: {e}"),
        }
    }
}

/// Create a styled dialog matching the thane dark theme.
fn styled_setup_dialog(
    parent: &gtk4::ApplicationWindow,
    width: i32,
    height: i32,
) -> (gtk4::Window, gtk4::Box) {
    let dialog = gtk4::Window::builder()
        .title("thane Setup")
        .transient_for(parent)
        .modal(true)
        .default_width(width)
        .default_height(height)
        .resizable(false)
        .build();
    dialog.add_css_class("thane-dialog");

    let header_bar = gtk4::HeaderBar::new();
    header_bar.add_css_class("thane-header");
    let title_label = gtk4::Label::new(Some("thane Setup"));
    title_label.add_css_class("thane-header-title");
    header_bar.set_title_widget(Some(&title_label));
    dialog.set_titlebar(Some(&header_bar));

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    vbox.add_css_class("thane-dialog-content");
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);
    vbox.set_halign(gtk4::Align::Center);
    vbox.set_valign(gtk4::Align::Center);
    dialog.set_child(Some(&vbox));

    (dialog, vbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_dependencies_returns_struct() {
        let status = check_dependencies();
        // We can't guarantee node/claude are installed in CI,
        // but the struct should always be constructed.
        let _ = status.has_node;
        let _ = status.has_claude;
    }
}
