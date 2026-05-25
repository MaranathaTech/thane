//! Linux systemd-user installer (`~/.config/systemd/user/thane-daemon.service`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};

pub const UNIT_NAME: &str = "thane-daemon.service";

pub fn unit_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("HOME directory unknown"))?;
    Ok(home
        .join(".config")
        .join("systemd")
        .join("user")
        .join(UNIT_NAME))
}

fn current_binary_path() -> Result<PathBuf> {
    std::env::current_exe().context("resolving current executable")
}

/// Build the systemd unit file contents for the given binary path.
pub fn unit_contents(binary: &Path) -> String {
    format!(
        "[Unit]
Description=thane terminal workspace manager daemon
After=default.target

[Service]
Type=simple
ExecStart={bin}
Restart=on-failure
RestartSec=2s
Environment=PATH=/usr/local/bin:%h/.local/bin:%h/.cargo/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
",
        bin = binary.display(),
    )
}

/// Install + enable + start the user service. Idempotent.
pub fn install() -> Result<PathBuf> {
    let binary = current_binary_path()?;
    let unit = unit_path()?;
    if let Some(parent) = unit.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&unit, unit_contents(&binary))
        .with_context(|| format!("writing unit {}", unit.display()))?;

    // Reload the user manager so it picks up our changes.
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    let status = Command::new("systemctl")
        .args(["--user", "enable", "--now", UNIT_NAME])
        .status()
        .context("invoking systemctl --user enable --now")?;
    if !status.success() {
        return Err(anyhow!(
            "systemctl --user enable --now exited with status {status}"
        ));
    }
    Ok(unit)
}

/// Stop and remove the user service.
pub fn uninstall() -> Result<()> {
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", UNIT_NAME])
        .status();
    let unit = unit_path()?;
    if unit.exists() {
        std::fs::remove_file(&unit)
            .with_context(|| format!("removing {}", unit.display()))?;
    }
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    Ok(())
}

/// Whether the user service unit is installed.
pub fn is_installed() -> bool {
    unit_path().map(|p| p.exists()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_includes_exec_start_and_restart() {
        let bin = Path::new("/usr/bin/thane-daemon");
        let unit = unit_contents(bin);
        assert!(unit.contains("ExecStart=/usr/bin/thane-daemon"), "{unit}");
        assert!(unit.contains("Restart=on-failure"), "{unit}");
        assert!(unit.contains("WantedBy=default.target"), "{unit}");
    }
}
