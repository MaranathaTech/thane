//! Landlock LSM enforcement for sandbox policies.
//!
//! Landlock is a Linux Security Module (available since Linux 5.13) that allows
//! unprivileged processes to restrict their own filesystem access. This module
//! translates thane sandbox policies into Landlock rulesets and applies them
//! to child processes at spawn time.
//!
//! The enforcement is applied in the `child_setup` callback of VTE's
//! `spawn_async`, which runs in the forked child before exec. This means
//! the restrictions apply to the shell and all its descendants.

use std::os::unix::io::RawFd;
use std::path::Path;

use thane_core::sandbox::{LandlockAccess, LandlockRule, SandboxPolicy};

// Landlock ABI version we target (v1 is the minimum, available since Linux 5.13).
const LANDLOCK_ABI_VERSION: u32 = 1;

// Landlock syscall numbers (x86_64).
#[cfg(target_arch = "x86_64")]
const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
#[cfg(target_arch = "x86_64")]
const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
#[cfg(target_arch = "x86_64")]
const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;

// Landlock syscall numbers (aarch64).
#[cfg(target_arch = "aarch64")]
const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
#[cfg(target_arch = "aarch64")]
const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
#[cfg(target_arch = "aarch64")]
const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;

// Landlock access flags for filesystem operations (ABI v1).
const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;

/// All filesystem access flags handled by Landlock ABI v1.
const LANDLOCK_ACCESS_FS_ALL: u64 = LANDLOCK_ACCESS_FS_EXECUTE
    | LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_READ_FILE
    | LANDLOCK_ACCESS_FS_READ_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_CHAR
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG
    | LANDLOCK_ACCESS_FS_MAKE_SOCK
    | LANDLOCK_ACCESS_FS_MAKE_FIFO
    | LANDLOCK_ACCESS_FS_MAKE_BLOCK
    | LANDLOCK_ACCESS_FS_MAKE_SYM;

/// Read-only access flags for directories (includes READ_DIR).
const LANDLOCK_ACCESS_FS_READ_DIR_FULL: u64 =
    LANDLOCK_ACCESS_FS_EXECUTE | LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR;

/// Read-only access flags for regular files (no READ_DIR — Landlock rejects it on non-dirs).
const LANDLOCK_ACCESS_FS_READ_FILE_ONLY: u64 =
    LANDLOCK_ACCESS_FS_EXECUTE | LANDLOCK_ACCESS_FS_READ_FILE;

/// Read-write access flags for directories (everything).
const LANDLOCK_ACCESS_FS_READWRITE_DIR: u64 = LANDLOCK_ACCESS_FS_ALL;

/// Read-write access flags for regular files (excludes dir-only operations).
const LANDLOCK_ACCESS_FS_READWRITE_FILE: u64 = LANDLOCK_ACCESS_FS_EXECUTE
    | LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_READ_FILE;

// Landlock rule type.
const LANDLOCK_RULE_PATH_BENEATH: libc::c_int = 1;

// Landlock create_ruleset flags.
const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;

/// Kernel ABI structs for Landlock syscalls.
///
/// These match the kernel's `struct landlock_ruleset_attr` and
/// `struct landlock_path_beneath_attr` exactly.
#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

#[repr(C)]
struct LandlockPathBeneathAttr {
    allowed_access: u64,
    parent_fd: RawFd,
}

/// Error type for Landlock operations.
#[derive(Debug, thiserror::Error)]
pub enum LandlockError {
    #[error("Landlock not supported on this kernel (requires Linux >= 5.13)")]
    NotSupported,
    #[error("Failed to create Landlock ruleset: {0}")]
    CreateRuleset(std::io::Error),
    #[error("Failed to add Landlock rule for path {path}: {source}")]
    AddRule {
        path: String,
        source: std::io::Error,
    },
    #[error("Failed to restrict self with Landlock: {0}")]
    RestrictSelf(std::io::Error),
    #[error("Failed to open path for Landlock rule: {path}: {source}")]
    OpenPath {
        path: String,
        source: std::io::Error,
    },
    #[error("Failed to set no_new_privs: {0}")]
    NoNewPrivs(std::io::Error),
}

/// Check if Landlock is supported on the running kernel.
pub fn is_landlock_supported() -> bool {
    // Query ABI version by calling create_ruleset with the VERSION flag and NULL attr.
    let abi = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<LandlockRulesetAttr>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    abi >= LANDLOCK_ABI_VERSION as libc::c_long
}

/// Apply a sandbox policy using Landlock LSM.
///
/// This function:
/// 1. Creates a Landlock ruleset that handles all filesystem access
/// 2. Adds rules for each allowed path in the policy
/// 3. Sets `PR_SET_NO_NEW_PRIVS` (required by Landlock)
/// 4. Restricts the current process with the ruleset
///
/// **IMPORTANT**: This must be called in a forked child process (e.g. in
/// VTE's `child_setup` callback), NOT in the parent process. Landlock
/// restrictions are permanent and inherited by all child processes.
pub fn apply_sandbox(policy: &SandboxPolicy) -> Result<(), LandlockError> {
    if !policy.enabled {
        return Ok(());
    }

    if !is_landlock_supported() {
        return Err(LandlockError::NotSupported);
    }

    let rules = policy.landlock_rules();
    if rules.is_empty() {
        return Ok(());
    }

    // Step 1: Create a ruleset that handles all filesystem access types.
    let ruleset_fd = create_ruleset(LANDLOCK_ACCESS_FS_ALL)?;

    // Step 2: Add a rule for each allowed path.
    for rule in &rules {
        if let Err(e) = add_path_rule(ruleset_fd, rule) {
            // Close the ruleset FD on error.
            unsafe { libc::close(ruleset_fd) };
            return Err(e);
        }
    }

    // Step 3: Set no_new_privs (required before landlock_restrict_self).
    set_no_new_privs()?;

    // Step 4: Restrict ourselves with the ruleset.
    let result = restrict_self(ruleset_fd);

    // Always close the ruleset FD.
    unsafe { libc::close(ruleset_fd) };

    result
}

/// Apply a sandbox policy, but don't fail if Landlock is unsupported.
/// Returns `true` if Landlock was successfully applied, `false` if unsupported.
pub fn try_apply_sandbox(policy: &SandboxPolicy) -> Result<bool, LandlockError> {
    if !policy.enabled {
        return Ok(false);
    }

    if !is_landlock_supported() {
        tracing::warn!("Landlock not supported on this kernel; sandbox policy will not be enforced at kernel level");
        return Ok(false);
    }

    apply_sandbox(policy)?;
    Ok(true)
}

/// Serialize a sandbox policy into a format that can be reconstructed in a
/// child process. Returns JSON bytes suitable for passing via an environment
/// variable or pipe.
pub fn serialize_policy(policy: &SandboxPolicy) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(policy)
}

/// Deserialize a sandbox policy from JSON bytes.
pub fn deserialize_policy(data: &[u8]) -> Result<SandboxPolicy, serde_json::Error> {
    serde_json::from_slice(data)
}

// --- Internal syscall wrappers ---

fn create_ruleset(handled_access_fs: u64) -> Result<RawFd, LandlockError> {
    let attr = LandlockRulesetAttr {
        handled_access_fs,
    };
    let fd = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const LandlockRulesetAttr,
            std::mem::size_of::<LandlockRulesetAttr>(),
            0u32,
        )
    };
    if fd < 0 {
        return Err(LandlockError::CreateRuleset(std::io::Error::last_os_error()));
    }
    Ok(fd as RawFd)
}

fn add_path_rule(ruleset_fd: RawFd, rule: &LandlockRule) -> Result<(), LandlockError> {
    let path = &rule.path;

    // Skip paths that don't exist — the agent may not have all default paths.
    if !path.exists() {
        tracing::debug!("Skipping non-existent Landlock path: {}", path.display());
        return Ok(());
    }

    let path_fd = open_path(path)?;

    // Landlock requires access flags compatible with the path type.
    // Applying READ_DIR to a regular file (or MAKE_DIR etc.) returns EINVAL.
    let is_dir = path.is_dir();
    let allowed_access = match (rule.access, is_dir) {
        (LandlockAccess::ReadOnly, true) => LANDLOCK_ACCESS_FS_READ_DIR_FULL,
        (LandlockAccess::ReadOnly, false) => LANDLOCK_ACCESS_FS_READ_FILE_ONLY,
        (LandlockAccess::ReadWrite, true) => LANDLOCK_ACCESS_FS_READWRITE_DIR,
        (LandlockAccess::ReadWrite, false) => LANDLOCK_ACCESS_FS_READWRITE_FILE,
    };

    let attr = LandlockPathBeneathAttr {
        allowed_access,
        parent_fd: path_fd,
    };

    let ret = unsafe {
        libc::syscall(
            SYS_LANDLOCK_ADD_RULE,
            ruleset_fd,
            LANDLOCK_RULE_PATH_BENEATH,
            &attr as *const LandlockPathBeneathAttr,
            0u32,
        )
    };

    unsafe { libc::close(path_fd) };

    if ret < 0 {
        return Err(LandlockError::AddRule {
            path: path.display().to_string(),
            source: std::io::Error::last_os_error(),
        });
    }

    Ok(())
}

fn open_path(path: &Path) -> Result<RawFd, LandlockError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        LandlockError::OpenPath {
            path: path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path contains null byte",
            ),
        }
    })?;

    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };

    if fd < 0 {
        return Err(LandlockError::OpenPath {
            path: path.display().to_string(),
            source: std::io::Error::last_os_error(),
        });
    }

    Ok(fd)
}

fn set_no_new_privs() -> Result<(), LandlockError> {
    let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if ret < 0 {
        return Err(LandlockError::NoNewPrivs(std::io::Error::last_os_error()));
    }
    Ok(())
}

fn restrict_self(ruleset_fd: RawFd) -> Result<(), LandlockError> {
    let ret = unsafe {
        libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, ruleset_fd, 0u32)
    };
    if ret < 0 {
        return Err(LandlockError::RestrictSelf(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Apply POSIX resource limits from a sandbox policy using setrlimit(2).
///
/// This should be called in the forked child process alongside Landlock enforcement.
/// Resource limits are inherited by child processes.
pub fn apply_resource_limits(policy: &SandboxPolicy) -> Result<(), ResourceLimitError> {
    if !policy.enabled {
        return Ok(());
    }

    let limits = policy.resource_limits();
    for limit in &limits {
        let resource = match limit.resource {
            thane_core::sandbox::RlimitResource::NoFile => libc::RLIMIT_NOFILE,
            thane_core::sandbox::RlimitResource::FSize => libc::RLIMIT_FSIZE,
            thane_core::sandbox::RlimitResource::Cpu => libc::RLIMIT_CPU,
        };

        let rlim = libc::rlimit {
            rlim_cur: limit.soft,
            rlim_max: limit.hard,
        };

        let ret = unsafe { libc::setrlimit(resource, &rlim) };
        if ret != 0 {
            return Err(ResourceLimitError::SetRlimit {
                resource: format!("{:?}", limit.resource),
                source: std::io::Error::last_os_error(),
            });
        }
    }

    Ok(())
}

/// Error type for resource limit operations.
#[derive(Debug, thiserror::Error)]
pub enum ResourceLimitError {
    #[error("Failed to set resource limit {resource}: {source}")]
    SetRlimit {
        resource: String,
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use thane_core::sandbox::SandboxPolicy;

    #[test]
    fn test_landlock_support_check() {
        // This just checks the function doesn't panic.
        // On CI machines without Landlock support, this will return false.
        let _supported = is_landlock_supported();
    }

    #[test]
    fn test_disabled_policy_is_noop() {
        let policy = SandboxPolicy::default();
        assert!(!policy.enabled);
        assert!(apply_sandbox(&policy).is_ok());
    }

    #[test]
    fn test_serialize_deserialize_policy() {
        let policy = SandboxPolicy::confined_to("/home/user/project");
        let data = serialize_policy(&policy).unwrap();
        let restored = deserialize_policy(&data).unwrap();
        assert_eq!(restored.root_dir, policy.root_dir);
        assert_eq!(restored.enabled, true);
    }

    #[test]
    fn test_access_flags_consistency() {
        // Verify read flags are a subset of readwrite flags for both dirs and files.
        assert_eq!(
            LANDLOCK_ACCESS_FS_READ_DIR_FULL & LANDLOCK_ACCESS_FS_READWRITE_DIR,
            LANDLOCK_ACCESS_FS_READ_DIR_FULL
        );
        assert_eq!(
            LANDLOCK_ACCESS_FS_READ_FILE_ONLY & LANDLOCK_ACCESS_FS_READWRITE_FILE,
            LANDLOCK_ACCESS_FS_READ_FILE_ONLY
        );
        // File flags must not include dir-only flags.
        assert_eq!(LANDLOCK_ACCESS_FS_READ_FILE_ONLY & LANDLOCK_ACCESS_FS_READ_DIR, 0);
        assert_eq!(LANDLOCK_ACCESS_FS_READWRITE_FILE & LANDLOCK_ACCESS_FS_READ_DIR, 0);
    }

    #[test]
    fn test_try_apply_disabled() {
        let policy = SandboxPolicy::default();
        let result = try_apply_sandbox(&policy).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_apply_resource_limits_disabled() {
        let policy = SandboxPolicy::default();
        assert!(!policy.enabled);
        assert!(apply_resource_limits(&policy).is_ok());
    }

    #[test]
    fn test_apply_resource_limits_no_limits() {
        let policy = SandboxPolicy::confined_to("/home/user/project");
        // Default policy has no resource limits set.
        assert!(policy.resource_limits().is_empty());
        assert!(apply_resource_limits(&policy).is_ok());
    }

    #[test]
    fn test_apply_resource_limits_nofile() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.max_open_files = Some(4096);

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].resource, thane_core::sandbox::RlimitResource::NoFile);
        assert_eq!(limits[0].soft, 4096);
        assert_eq!(limits[0].hard, 4096);

        // Actually apply it — RLIMIT_NOFILE of 4096 is safe.
        assert!(apply_resource_limits(&policy).is_ok());
    }

    #[test]
    fn test_apply_resource_limits_fsize() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.max_write_bytes = Some(100 * 1024 * 1024); // 100 MB

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].resource, thane_core::sandbox::RlimitResource::FSize);
    }

    #[test]
    fn test_apply_resource_limits_cpu() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.cpu_time_limit = Some(300); // 5 minutes

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].resource, thane_core::sandbox::RlimitResource::Cpu);
        assert_eq!(limits[0].soft, 300);
        assert_eq!(limits[0].hard, 310); // 10-second grace period
    }

    #[test]
    fn test_apply_resource_limits_all() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.max_open_files = Some(1024);
        policy.max_write_bytes = Some(50 * 1024 * 1024);
        policy.cpu_time_limit = Some(60);

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 3);
    }
}
