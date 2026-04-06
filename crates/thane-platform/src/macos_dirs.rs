use std::path::PathBuf;

use crate::traits::PlatformDirs;

const APP_NAME: &str = "thane";

/// macOS directory resolution using ~/Library paths.
pub struct MacosDirs;

impl PlatformDirs for MacosDirs {
    fn config_dir(&self) -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join("Library/Application Support")
            })
            .join(APP_NAME)
    }

    fn data_dir(&self) -> PathBuf {
        // On macOS, Application Support is used for both config and data.
        dirs::data_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join("Library/Application Support")
            })
            .join(APP_NAME)
    }

    fn cache_dir(&self) -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join("Library/Caches")
            })
            .join(APP_NAME)
    }

    fn runtime_dir(&self) -> PathBuf {
        // macOS doesn't have XDG_RUNTIME_DIR; use Application Support/thane/run.
        self.data_dir().join("run")
    }

    fn sessions_dir(&self) -> PathBuf {
        self.data_dir().join("sessions")
    }

    fn socket_path(&self) -> PathBuf {
        self.runtime_dir().join("thane.sock")
    }
}

impl MacosDirs {
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
        let dirs = MacosDirs;
        assert!(dirs.config_dir().to_str().unwrap().contains(APP_NAME));
        assert!(dirs.data_dir().to_str().unwrap().contains(APP_NAME));
        assert!(dirs.cache_dir().to_str().unwrap().contains(APP_NAME));
        assert!(dirs.sessions_dir().to_str().unwrap().contains(APP_NAME));
    }

    #[test]
    fn test_socket_path() {
        let dirs = MacosDirs;
        let socket = dirs.socket_path();
        assert!(socket.to_str().unwrap().ends_with("thane.sock"));
    }

    #[test]
    fn test_runtime_dir_under_data() {
        let dirs = MacosDirs;
        let runtime = dirs.runtime_dir();
        let data = dirs.data_dir();
        assert!(runtime.starts_with(&data));
    }

    #[test]
    fn test_setup_sentinel_path() {
        let dirs = MacosDirs;
        let sentinel = dirs.setup_sentinel();
        assert!(sentinel.to_str().unwrap().ends_with(".setup-complete"));
        assert!(sentinel.to_str().unwrap().contains(APP_NAME));
    }
}
