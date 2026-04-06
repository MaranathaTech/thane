use std::path::PathBuf;

/// Platform-specific directory resolution.
pub trait PlatformDirs {
    /// Config directory (e.g. ~/.config/thane)
    fn config_dir(&self) -> PathBuf;
    /// Data directory (e.g. ~/.local/share/thane)
    fn data_dir(&self) -> PathBuf;
    /// Cache directory (e.g. ~/.cache/thane)
    fn cache_dir(&self) -> PathBuf;
    /// Runtime directory (e.g. /run/user/1000/thane)
    fn runtime_dir(&self) -> PathBuf;
    /// Session storage directory
    fn sessions_dir(&self) -> PathBuf;
    /// Socket path for IPC
    fn socket_path(&self) -> PathBuf;
}

/// Platform-specific desktop notification sender.
pub trait DesktopNotifier {
    fn send_notification(
        &self,
        title: &str,
        body: &str,
        urgency: NotifyUrgency,
    ) -> Result<(), Box<dyn std::error::Error>>;
}

/// Urgency level for desktop notifications.
#[derive(Debug, Clone, Copy)]
pub enum NotifyUrgency {
    Low,
    Normal,
    Critical,
}

/// Platform-specific process ancestry checking.
pub trait ProcessAncestryChecker {
    /// Check if `child_pid` is a descendant of `ancestor_pid`.
    fn is_descendant(&self, child_pid: u32, ancestor_pid: u32) -> bool;

    /// Get child PIDs of a process (direct children).
    fn child_pids(&self, pid: u32) -> Vec<u32>;
}

/// Platform-specific port scanner.
pub trait PortScanner {
    /// Scan for listening TCP ports, optionally filtered by PIDs.
    fn scan_listening_ports(&self, pids: &[u32]) -> Vec<u16>;
}
