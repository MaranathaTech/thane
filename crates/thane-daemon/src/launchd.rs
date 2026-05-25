//! macOS LaunchAgent installer (`~/Library/LaunchAgents/com.thane.daemon.plist`).
//!
//! Idempotent: install rewrites the plist with the current binary path and
//! re-loads it via `launchctl`. Uninstall removes the plist and unloads it.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};

/// LaunchAgent label.
pub const LABEL: &str = "com.thane.daemon";

/// Path to `~/Library/LaunchAgents/com.thane.daemon.plist`.
pub fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("HOME directory unknown"))?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

/// Logs directory `~/Library/Logs/thane`.
fn logs_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("HOME directory unknown"))?;
    Ok(home.join("Library").join("Logs").join("thane"))
}

/// Determine which `thane-daemon` binary to invoke. Resolves the current
/// executable so `thane-daemon --install-launch-agent` always wires the plist
/// to the binary that's installing it (works for both /usr/local/bin and
/// Contents/MacOS layouts).
fn current_binary_path() -> Result<PathBuf> {
    std::env::current_exe().context("resolving current executable")
}

/// Build the plist XML for the given binary path.
pub fn plist_contents(binary: &Path) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/user".to_string());
    let log = format!("{home}/Library/Logs/thane/daemon.log");
    let path_env = format!(
        "/opt/homebrew/bin:/usr/local/bin:{home}/.local/bin:{home}/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin"
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path_env}</string>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#,
        bin = binary.display(),
    )
}

/// Install the LaunchAgent. Idempotent.
pub fn install() -> Result<PathBuf> {
    let binary = current_binary_path()?;
    let plist = plist_path()?;
    let logs = logs_dir()?;

    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::create_dir_all(&logs)
        .with_context(|| format!("creating logs dir {}", logs.display()))?;

    let contents = plist_contents(&binary);
    std::fs::write(&plist, contents)
        .with_context(|| format!("writing plist {}", plist.display()))?;

    // Unload first in case an older copy is loaded; ignore errors.
    let _ = Command::new("launchctl").arg("unload").arg(&plist).status();
    let status = Command::new("launchctl")
        .arg("load")
        .arg("-w")
        .arg(&plist)
        .status()
        .context("invoking launchctl load")?;
    if !status.success() {
        return Err(anyhow!(
            "launchctl load exited with status {status} for {}",
            plist.display()
        ));
    }
    Ok(plist)
}

/// Uninstall the LaunchAgent if present. No-op when nothing is installed.
pub fn uninstall() -> Result<()> {
    let plist = plist_path()?;
    if plist.exists() {
        let _ = Command::new("launchctl").arg("unload").arg(&plist).status();
        std::fs::remove_file(&plist)
            .with_context(|| format!("removing {}", plist.display()))?;
    }
    Ok(())
}

/// Whether the LaunchAgent plist is installed.
pub fn is_installed() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn plist_includes_label_and_binary_path() {
        let bin = Path::new("/Applications/thane.app/Contents/MacOS/thane-daemon");
        let xml = plist_contents(bin);
        assert!(xml.contains("com.thane.daemon"), "missing label: {xml}");
        assert!(
            xml.contains("/Applications/thane.app/Contents/MacOS/thane-daemon"),
            "missing binary path: {xml}"
        );
        assert!(xml.contains("<key>RunAtLoad</key>"));
        assert!(xml.contains("<key>KeepAlive</key>"));
        assert!(xml.contains("<key>SuccessfulExit</key>"));
        // KeepAlive must use false for SuccessfulExit so the daemon only
        // restarts on crash, not on clean exit.
        assert!(xml.contains("<key>SuccessfulExit</key>\n        <false/>"));
    }

    #[test]
    fn plist_path_is_under_user_launch_agents() {
        // Skip in environments without HOME (CI sandboxes).
        let Ok(path) = plist_path() else { return };
        let s = path.to_string_lossy();
        assert!(s.contains("Library/LaunchAgents"), "unexpected path: {s}");
        assert!(s.ends_with("com.thane.daemon.plist"));
    }
}
