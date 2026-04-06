use std::path::PathBuf;

use crate::traits::PlatformDirs;

const APP_NAME: &str = "thane";

/// XDG-based directory resolution for Linux.
pub struct LinuxDirs;

impl PlatformDirs for LinuxDirs {
    fn config_dir(&self) -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join(APP_NAME)
    }

    fn data_dir(&self) -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("~/.local/share"))
            .join(APP_NAME)
    }

    fn cache_dir(&self) -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("~/.cache"))
            .join(APP_NAME)
    }

    fn runtime_dir(&self) -> PathBuf {
        dirs::runtime_dir()
            .unwrap_or_else(|| {
                // Fallback: /tmp/thane-<uid>
                let uid = nix::unistd::getuid();
                PathBuf::from(format!("/tmp/{APP_NAME}-{uid}"))
            })
            .join(APP_NAME)
    }

    fn sessions_dir(&self) -> PathBuf {
        self.data_dir().join("sessions")
    }

    fn socket_path(&self) -> PathBuf {
        self.runtime_dir().join("thane.sock")
    }
}

impl LinuxDirs {
    /// Plans directory for queued plan files.
    pub fn plans_dir(&self) -> PathBuf {
        self.data_dir().join("plans")
    }

    /// Ensure all required directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.config_dir())?;
        std::fs::create_dir_all(self.data_dir())?;
        std::fs::create_dir_all(self.cache_dir())?;
        std::fs::create_dir_all(self.runtime_dir())?;
        std::fs::create_dir_all(self.sessions_dir())?;
        std::fs::create_dir_all(self.plans_dir())?;
        Ok(())
    }

    /// Path to the setup-complete sentinel file.
    pub fn setup_sentinel(&self) -> PathBuf {
        self.data_dir().join(".setup-complete")
    }

    /// Whether one-time setup (CLAUDE.md injection) has been completed.
    pub fn is_setup_complete(&self) -> bool {
        self.setup_sentinel().exists()
    }

    /// Mark one-time setup as complete by writing a timestamp sentinel.
    pub fn mark_setup_complete(&self) -> std::io::Result<()> {
        let path = self.setup_sentinel();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, "done")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dirs_contain_app_name() {
        let dirs = LinuxDirs;
        assert!(dirs.config_dir().to_str().unwrap().contains(APP_NAME));
        assert!(dirs.data_dir().to_str().unwrap().contains(APP_NAME));
        assert!(dirs.cache_dir().to_str().unwrap().contains(APP_NAME));
        assert!(dirs.sessions_dir().to_str().unwrap().contains(APP_NAME));
    }

    #[test]
    fn test_socket_path() {
        let dirs = LinuxDirs;
        let socket = dirs.socket_path();
        assert!(socket.to_str().unwrap().ends_with("thane.sock"));
    }

    #[test]
    fn test_setup_sentinel_path() {
        let dirs = LinuxDirs;
        let sentinel = dirs.setup_sentinel();
        assert!(sentinel.to_str().unwrap().ends_with(".setup-complete"));
        assert!(sentinel.to_str().unwrap().contains(APP_NAME));
    }
}
