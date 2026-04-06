//! Mount namespace isolation for hiding paths from sandboxed processes.
//!
//! When enabled, creates a new mount namespace and bind-mounts an empty tmpfs
//! over denied paths, making them appear empty rather than just access-denied.
//! This is stronger than Landlock alone because the paths are invisible
//! to directory listings.
//!
//! Requires either:
//! - `CAP_SYS_ADMIN` (root), or
//! - User namespace support (`/proc/sys/kernel/unprivileged_userns_clone = 1`)
//!
//! Falls back gracefully if unavailable.

use thane_core::sandbox::SandboxPolicy;

/// Error type for namespace operations.
#[derive(Debug, thiserror::Error)]
pub enum NamespaceError {
    #[error("Failed to unshare mount namespace: {0}")]
    Unshare(std::io::Error),
    #[error("Failed to mount tmpfs over {path}: {source}")]
    Mount {
        path: String,
        source: std::io::Error,
    },
    #[error("Mount namespace isolation not supported")]
    NotSupported,
}

/// Check if user namespaces are available (needed for unprivileged mount namespaces).
pub fn is_userns_supported() -> bool {
    // Check if unprivileged user namespaces are enabled.
    std::fs::read_to_string("/proc/sys/kernel/unprivileged_userns_clone")
        .map(|s| s.trim() == "1")
        .unwrap_or_else(|_| {
            // On some kernels, the file doesn't exist but user namespaces are available.
            // Try to check /proc/sys/user/max_user_namespaces instead.
            std::fs::read_to_string("/proc/sys/user/max_user_namespaces")
                .map(|s| s.trim().parse::<u64>().unwrap_or(0) > 0)
                .unwrap_or(false)
        })
}

/// Apply mount namespace isolation for denied paths.
///
/// This function:
/// 1. Creates a new user + mount namespace via `unshare(2)`
/// 2. Mounts an empty tmpfs over each denied path, hiding its contents
///
/// **IMPORTANT**: Must be called in the forked child process.
/// Should be called BEFORE Landlock (Landlock needs to see the mount tree).
pub fn apply_mount_namespace(policy: &SandboxPolicy) -> Result<(), NamespaceError> {
    if !policy.enabled || policy.denied_paths.is_empty() {
        return Ok(());
    }

    // Create a new user namespace + mount namespace.
    // CLONE_NEWUSER gives us CAP_SYS_ADMIN within the namespace,
    // allowing us to mount without actual root privileges.
    let flags = libc::CLONE_NEWNS | libc::CLONE_NEWUSER;
    let ret = unsafe { libc::unshare(flags) };

    if ret != 0 {
        let err = std::io::Error::last_os_error();
        // EPERM means user namespaces are disabled.
        if err.raw_os_error() == Some(libc::EPERM) {
            return Err(NamespaceError::NotSupported);
        }
        return Err(NamespaceError::Unshare(err));
    }

    // Write uid/gid mappings for the new user namespace.
    // Map our UID/GID 1:1 so file access works normally.
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    // Write uid_map: map uid 0 in the namespace to our real uid.
    let uid_map = format!("0 {uid} 1\n");
    let _ = std::fs::write("/proc/self/uid_map", uid_map);

    // Disable setgroups (required before writing gid_map).
    let _ = std::fs::write("/proc/self/setgroups", "deny");

    // Write gid_map.
    let gid_map = format!("0 {gid} 1\n");
    let _ = std::fs::write("/proc/self/gid_map", gid_map);

    // Make the mount namespace private (prevent mount propagation to parent).
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            c"/".as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        // Non-fatal: propagation control is nice but not required.
        tracing::debug!("Failed to set mount namespace to private: {}", std::io::Error::last_os_error());
    }

    // Mount empty tmpfs over each denied path.
    for path in &policy.denied_paths {
        if !path.exists() {
            continue;
        }

        let path_cstr = match std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let ret = unsafe {
            libc::mount(
                c"tmpfs".as_ptr(),
                path_cstr.as_ptr(),
                c"tmpfs".as_ptr(),
                libc::MS_RDONLY | libc::MS_NOEXEC | libc::MS_NOSUID | libc::MS_NODEV,
                c"size=0".as_ptr() as *const libc::c_void,
            )
        };

        if ret != 0 {
            let err = std::io::Error::last_os_error();
            tracing::warn!("Failed to hide path {}: {err}", path.display());
            // Non-fatal in most modes — Landlock still blocks access.
        } else {
            tracing::debug!("Hidden path via mount namespace: {}", path.display());
        }
    }

    Ok(())
}

/// Try to apply mount namespace isolation. Returns `true` if applied, `false` if unavailable.
pub fn try_apply_mount_namespace(policy: &SandboxPolicy) -> Result<bool, NamespaceError> {
    if !policy.enabled || policy.denied_paths.is_empty() {
        return Ok(false);
    }

    match apply_mount_namespace(policy) {
        Ok(()) => Ok(true),
        Err(NamespaceError::NotSupported) => {
            tracing::info!("Mount namespace isolation not available (user namespaces disabled)");
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_userns_support_check() {
        // Just verify it doesn't panic.
        let _supported = is_userns_supported();
    }

    #[test]
    fn test_disabled_policy_is_noop() {
        let policy = SandboxPolicy::default();
        assert!(apply_mount_namespace(&policy).is_ok());
    }

    #[test]
    fn test_empty_denied_paths_is_noop() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.denied_paths = Vec::new();
        assert!(apply_mount_namespace(&policy).is_ok());
    }

    #[test]
    fn test_try_apply_disabled() {
        let policy = SandboxPolicy::default();
        let result = try_apply_mount_namespace(&policy).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_try_apply_no_denied_paths() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.denied_paths = Vec::new();
        let result = try_apply_mount_namespace(&policy).unwrap();
        assert!(!result);
    }
}
