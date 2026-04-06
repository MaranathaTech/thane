pub mod claude_md;
pub mod traits;

// Platform-specific modules.

#[cfg(target_os = "linux")]
pub mod dirs;
#[cfg(target_os = "linux")]
pub mod landlock;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod namespace;
#[cfg(target_os = "linux")]
pub mod seccomp;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub mod macos_dirs;
#[cfg(target_os = "macos")]
pub mod sandbox_macos;

// Shared modules (work on both platforms).
pub mod pidlock;

// Re-export Linux implementations as the default platform on Linux.
#[cfg(target_os = "linux")]
pub use dirs::LinuxDirs;
#[cfg(target_os = "linux")]
pub use landlock::{apply_resource_limits, apply_sandbox, is_landlock_supported, try_apply_sandbox};
#[cfg(target_os = "linux")]
pub use linux::{LinuxNotifier, LinuxPortScanner, LinuxProcessChecker};
#[cfg(target_os = "linux")]
pub use namespace::try_apply_mount_namespace;
#[cfg(target_os = "linux")]
pub use seccomp::{apply_seccomp, is_seccomp_supported};

// Re-export macOS implementations on macOS.
#[cfg(target_os = "macos")]
pub use macos::{MacosNotifier, MacosPortScanner, MacosProcessChecker};
#[cfg(target_os = "macos")]
pub use macos_dirs::MacosDirs;
#[cfg(target_os = "macos")]
pub use sandbox_macos::{
    apply_resource_limits, apply_sandbox, apply_seccomp, generate_sandbox_command,
    generate_seatbelt_profile, is_sandbox_supported, is_seccomp_supported,
    try_apply_mount_namespace, try_apply_sandbox,
};
