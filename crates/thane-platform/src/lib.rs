pub mod claude_md;
pub mod secrets;
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
#[cfg(target_os = "linux")]
pub mod secrets_linux;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
pub mod macos_dirs;
#[cfg(target_os = "macos")]
pub mod sandbox_macos;
#[cfg(target_os = "macos")]
pub mod secrets_macos;

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
#[cfg(target_os = "linux")]
pub use secrets_linux::LinuxSecretStore;

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
#[cfg(target_os = "macos")]
pub use secrets_macos::MacosSecretStore;

// Cross-platform default secret store selector. Returns a boxed trait object so
// downstream crates (thane-core, thane-cli) don't have to know about
// platform-specific types.
#[cfg(target_os = "linux")]
pub fn default_secret_store() -> Box<dyn thane_core::secret_store::SecretStore> {
    Box::new(LinuxSecretStore::new())
}

#[cfg(target_os = "macos")]
pub fn default_secret_store() -> Box<dyn thane_core::secret_store::SecretStore> {
    Box::new(MacosSecretStore::new())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn default_secret_store() -> Box<dyn thane_core::secret_store::SecretStore> {
    Box::new(thane_core::secret_store::MemorySecretStore::new())
}

/// Human-readable name of the secret-store backend on this platform.
///
/// Used in the audit-encryption UI tooltip so the operator can see at a glance
/// where their audit AES key lives. Linux can fall through Secret Service to
/// an encrypted-file fallback at runtime, so we surface both possibilities.
pub fn default_secret_store_backend_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Keychain"
    }
    #[cfg(target_os = "linux")]
    {
        "Secret Service / local fallback"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "in-memory (test) store"
    }
}
