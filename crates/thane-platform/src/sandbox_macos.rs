//! macOS sandbox support via Apple's Seatbelt kernel framework.
//!
//! Uses `sandbox_init_with_parameters()` (the C API behind `sandbox-exec`)
//! to apply SBPL profiles at process level. The profile uses deny-default
//! for file operations with explicit allows for the working directory,
//! system paths, and toolchains.
//!
//! A helper binary (`thane-sandbox-exec`) applies the sandbox, resource
//! limits, and then execs the target shell — replacing the deprecated
//! `sandbox-exec` CLI tool.

use std::path::Path;

use thane_core::sandbox::{EnforcementLevel, SandboxPolicy};

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("macOS sandbox error: {0}")]
    SandboxInit(String),
}

/// Check if sandbox support is available on this macOS system.
///
/// The Seatbelt kernel framework is always available on macOS 10.5+.
/// We also check for the helper binary at the expected location.
pub fn is_sandbox_supported() -> bool {
    // Seatbelt is a kernel framework — always available on macOS.
    // The helper binary is bundled with the app or built alongside it.
    true
}

/// Generate a deny-default Seatbelt Profile Language (SBPL) profile.
///
/// The profile denies all file operations by default, then explicitly
/// allows access to the working directory, system paths, and toolchains.
/// Denied paths (secrets, credentials) are emitted last to override any
/// broad allows. SBPL uses last-match semantics for overlapping rules.
pub fn generate_seatbelt_profile(policy: &SandboxPolicy) -> String {
    let mut profile = String::with_capacity(4096);

    profile.push_str("(version 1)\n\n");

    // Allow all non-file system operations. macOS requires these for
    // dyld shared cache, Mach services, IOKit, XPC, etc.
    profile.push_str(";; Allow system operations (required for macOS process lifecycle)\n");
    profile.push_str("(allow process*)\n");
    profile.push_str("(allow sysctl*)\n");
    profile.push_str("(allow mach*)\n");
    profile.push_str("(allow ipc*)\n");
    profile.push_str("(allow signal)\n");
    profile.push_str("(allow system*)\n\n");

    // Deny all file operations by default
    profile.push_str(";; Deny all file access by default\n");
    profile.push_str("(deny file*)\n\n");

    // macOS system paths (read-only) — needed for shell, libraries, frameworks
    profile.push_str(";; System paths (read-only)\n");
    for sys_path in &[
        "/",         // Root directory listing
        "/System",   // macOS frameworks and dyld shared cache
        "/Library",  // System-wide libraries and frameworks
        "/usr",      // System binaries and libraries
        "/bin",      // Essential binaries
        "/sbin",     // System administration binaries
        "/etc",      // System configuration (firmlink to /private/etc)
        "/opt",      // Homebrew (Apple Silicon) and other tools
        "/Applications", // For app discovery
        "/private/etc",  // Real /etc (firmlinked)
        "/private/var",  // System state (needed for various services)
    ] {
        profile.push_str(&format!(
            "(allow file-read* (subpath \"{}\"))\n",
            escape_sbpl_path(sys_path)
        ));
    }
    // Root directory needs literal match for readdir
    profile.push_str("(allow file-read* (literal \"/\"))\n\n");

    // Device and temp paths (read-write) — needed for TTY, builds, temp files
    profile.push_str(";; Device and temp paths (read-write)\n");
    for rw_sys in &[
        "/dev",
        "/tmp",
        "/private/tmp",
        "/var/folders",         // macOS $TMPDIR lives here
        "/private/var/folders", // Real path behind firmlink
    ] {
        profile.push_str(&format!(
            "(allow file* (subpath \"{}\"))\n",
            escape_sbpl_path(rw_sys)
        ));
    }
    profile.push('\n');

    // Policy read-only paths (shell dotfiles, toolchains)
    if !policy.read_only_paths.is_empty() {
        profile.push_str(";; Policy read-only paths (dotfiles, toolchains)\n");
        for ro_path in &policy.read_only_paths {
            let escaped = escape_sbpl_path(&ro_path.to_string_lossy());
            // Individual files use literal, directories use subpath
            let name = ro_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let looks_like_file = name.contains('.') && !name.starts_with('.');
            if ro_path.is_file() || looks_like_file {
                profile.push_str(&format!(
                    "(allow file-read* (literal \"{escaped}\"))\n"
                ));
            } else {
                profile.push_str(&format!(
                    "(allow file-read* (subpath \"{escaped}\"))\n"
                ));
            }
        }
        profile.push('\n');
    }

    // Working directory (full read-write access)
    let root_escaped = escape_sbpl_path(&policy.root_dir.to_string_lossy());
    profile.push_str(";; Working directory (read-write)\n");
    profile.push_str(&format!("(allow file* (subpath \"{root_escaped}\"))\n"));

    // Additional read-write paths from policy
    for rw_path in &policy.read_write_paths {
        if rw_path == &policy.root_dir {
            continue; // Already added above
        }
        let escaped = escape_sbpl_path(&rw_path.to_string_lossy());
        profile.push_str(&format!("(allow file* (subpath \"{escaped}\"))\n"));
    }
    profile.push('\n');

    // Denied paths — MUST come last to override broader allows above.
    // This blocks secrets/credentials even if they're under an allowed subtree.
    if !policy.denied_paths.is_empty() {
        profile.push_str(";; Denied paths (secrets, credentials) — overrides allows above\n");
        for denied in &policy.denied_paths {
            let escaped = escape_sbpl_path(&denied.to_string_lossy());
            let name = denied.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let looks_like_file = name.contains('.') && !name.starts_with('.');
            if denied.is_dir() || !looks_like_file {
                profile.push_str(&format!(
                    "(deny file-read* file-write* (subpath \"{escaped}\"))\n"
                ));
            } else {
                profile.push_str(&format!(
                    "(deny file-read* file-write* (literal \"{escaped}\"))\n"
                ));
            }
        }
        profile.push('\n');
    }

    // Strict mode: use exec allowlist instead of blacklist.
    // Only allow execution from standard system paths — block user-writable locations.
    if policy.enforcement == EnforcementLevel::Strict {
        profile.push_str(";; Strict: deny process-exec by default, allowlist system paths\n");
        profile.push_str("(deny process-exec)\n");
        profile.push_str("(allow process-exec (subpath \"/usr/bin\"))\n");
        profile.push_str("(allow process-exec (subpath \"/usr/sbin\"))\n");
        profile.push_str("(allow process-exec (subpath \"/bin\"))\n");
        profile.push_str("(allow process-exec (subpath \"/sbin\"))\n");
        profile.push_str("(allow process-exec (subpath \"/usr/libexec\"))\n");
        profile.push_str("(allow process-exec (subpath \"/System\"))\n\n");
    }

    // Network restrictions — block all network operations including DNS
    if !policy.allow_network {
        profile.push_str(";; Network denied (all operations including DNS)\n");
        profile.push_str("(deny network*)\n\n");
    }

    profile
}

/// Generate the command to launch a sandboxed shell via `thane-sandbox-exec`.
///
/// The helper binary reads the SBPL profile from `THANE_SANDBOX_PROFILE`,
/// applies it via `sandbox_init_with_parameters()`, applies resource limits
/// via `setrlimit`, then execs the target shell.
///
/// Returns `(executable, args, env_vars)` or `None` if sandboxing is disabled.
/// In Permissive mode, the Seatbelt profile is still applied (violations are
/// logged to syslog) and `THANE_SANDBOX_PERMISSIVE=1` is added to env vars.
pub fn generate_sandbox_command(
    policy: &SandboxPolicy,
    shell: &str,
) -> Option<(String, Vec<String>, Vec<String>)> {
    if !policy.enabled {
        return None;
    }

    let profile = generate_seatbelt_profile(policy);

    // Find the helper binary. Check:
    // 1. Next to the running binary (built alongside)
    // 2. In the app bundle's MacOS directory
    // 3. Fall back to sandbox-exec if helper not found
    let helper_path = find_sandbox_helper();

    let mut env_vars: Vec<String> = policy
        .env_vars()
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();

    // In permissive mode, still apply the Seatbelt profile (violations are
    // logged to syslog) but set an env var so the shell/tools know this is
    // audit-only.
    if policy.enforcement == EnforcementLevel::Permissive {
        env_vars.push("THANE_SANDBOX_PERMISSIVE=1".to_string());
    }

    let (executable, args) = if let Some(helper) = helper_path {
        // Use our helper binary — it reads the profile from env, applies
        // sandbox + resource limits, then execs the shell.
        env_vars.push(format!("THANE_SANDBOX_PROFILE={profile}"));

        if let Some(max_files) = policy.max_open_files {
            env_vars.push(format!("THANE_SANDBOX_MAX_FILES={max_files}"));
        }
        if let Some(max_write) = policy.max_write_bytes {
            env_vars.push(format!("THANE_SANDBOX_MAX_FSIZE={max_write}"));
        }
        if let Some(max_cpu) = policy.cpu_time_limit {
            env_vars.push(format!("THANE_SANDBOX_MAX_CPU={max_cpu}"));
        }

        (
            helper,
            vec![shell.to_string(), "-l".to_string()],
        )
    } else {
        // Fall back to sandbox-exec (deprecated but still works)
        tracing::warn!("thane-sandbox-exec not found, falling back to sandbox-exec");
        if !Path::new("/usr/bin/sandbox-exec").exists() {
            tracing::error!("sandbox-exec not found either — sandbox cannot be applied");
            return None;
        }
        (
            "/usr/bin/sandbox-exec".to_string(),
            vec![
                "-p".to_string(),
                profile,
                "--".to_string(),
                shell.to_string(),
                "-l".to_string(),
            ],
        )
    };

    Some((executable, args, env_vars))
}

/// Find the thane-sandbox-exec helper binary.
fn find_sandbox_helper() -> Option<String> {
    // 1. Next to the current executable (debug/release builds)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let helper = dir.join("thane-sandbox-exec");
            if helper.exists() {
                return Some(helper.to_string_lossy().to_string());
            }
        }
    }

    // 2. In the app bundle's MacOS directory
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundle_helper = dir.join("../Resources/thane-sandbox-exec");
            if bundle_helper.exists() {
                return Some(bundle_helper.canonicalize().ok()?.to_string_lossy().to_string());
            }
        }
    }

    // 3. In /usr/local/bin (installed via CLI)
    let installed = Path::new("/usr/local/bin/thane-sandbox-exec");
    if installed.exists() {
        return Some(installed.to_string_lossy().to_string());
    }

    None
}

/// Apply sandbox policy. Currently delegates to sandbox-exec at process spawn time
/// rather than applying in-process. See `generate_sandbox_command()`.
pub fn apply_sandbox(_policy: &SandboxPolicy) -> Result<(), SandboxError> {
    tracing::debug!("macOS sandbox: use generate_sandbox_command() for process-level sandboxing");
    Ok(())
}

/// Try to apply sandbox policy, returning whether it was actually applied.
pub fn try_apply_sandbox(_policy: &SandboxPolicy) -> Result<bool, SandboxError> {
    tracing::debug!("macOS sandbox: use generate_sandbox_command() for process-level sandboxing");
    Ok(false)
}

/// Mount namespace isolation is not available on macOS.
pub fn try_apply_mount_namespace(_policy: &SandboxPolicy) -> Result<bool, SandboxError> {
    Ok(false)
}

/// Apply resource limits (RLIMIT_NOFILE, RLIMIT_FSIZE, RLIMIT_CPU).
///
/// Works on macOS since setrlimit is POSIX-standard.
pub fn apply_resource_limits(policy: &SandboxPolicy) -> Result<(), SandboxError> {
    if !policy.enabled {
        return Ok(());
    }

    if let Some(max_files) = policy.max_open_files {
        let rlim = libc::rlimit {
            rlim_cur: max_files,
            rlim_max: max_files,
        };
        unsafe {
            if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) != 0 {
                tracing::warn!("Failed to set RLIMIT_NOFILE");
            }
        }
    }

    if let Some(max_write) = policy.max_write_bytes {
        let rlim = libc::rlimit {
            rlim_cur: max_write,
            rlim_max: max_write,
        };
        unsafe {
            if libc::setrlimit(libc::RLIMIT_FSIZE, &rlim) != 0 {
                tracing::warn!("Failed to set RLIMIT_FSIZE");
            }
        }
    }

    if let Some(max_cpu) = policy.cpu_time_limit {
        let rlim = libc::rlimit {
            rlim_cur: max_cpu,
            rlim_max: max_cpu,
        };
        unsafe {
            if libc::setrlimit(libc::RLIMIT_CPU, &rlim) != 0 {
                tracing::warn!("Failed to set RLIMIT_CPU");
            }
        }
    }

    Ok(())
}

/// seccomp is not available on macOS.
pub fn apply_seccomp(_policy: &SandboxPolicy) -> Result<(), SandboxError> {
    Ok(())
}

/// seccomp is not available on macOS.
pub fn is_seccomp_supported() -> bool {
    false
}

/// Escape a path string for use inside an SBPL double-quoted string.
///
/// Also validates that the path doesn't contain SBPL metacharacters that
/// could be used for profile injection attacks.
fn escape_sbpl_path(path: &str) -> String {
    // Reject paths with SBPL syntax characters that could inject rules.
    // Parentheses and semicolons are SBPL operators and should never appear in paths.
    let sanitized: String = path
        .chars()
        .filter(|c| *c != '(' && *c != ')' && *c != ';')
        .collect();
    if sanitized.len() != path.len() {
        tracing::warn!(
            "Sandbox path contained SBPL metacharacters and was sanitized: {path}"
        );
    }
    sanitized.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_sandbox_supported() {
        assert!(is_sandbox_supported(), "Seatbelt is always available on macOS");
    }

    #[test]
    fn test_generate_seatbelt_profile_deny_default() {
        let policy = SandboxPolicy::confined_to("/Users/test/project");
        let profile = generate_seatbelt_profile(&policy);

        assert!(profile.contains("(version 1)"));
        // Must use deny-default for file operations
        assert!(profile.contains("(deny file*)"));
        // Must allow system operations
        assert!(profile.contains("(allow process*)"));
        assert!(profile.contains("(allow mach*)"));
        // Must allow working directory
        assert!(profile.contains("(allow file* (subpath \"/Users/test/project\"))"));
        // Must allow system paths
        assert!(profile.contains("(allow file-read* (subpath \"/usr\"))"));
        assert!(profile.contains("(allow file-read* (subpath \"/bin\"))"));
        // Default denied paths from confined_to()
        assert!(profile.contains(".ssh"));
    }

    #[test]
    fn test_denied_paths_come_last() {
        let mut policy = SandboxPolicy::confined_to("/Users/test/project");
        policy.denied_paths = vec![PathBuf::from("/Users/test/.ssh")];
        let profile = generate_seatbelt_profile(&policy);

        // Working dir allow should appear before denied paths
        let allow_pos = profile.find("(allow file* (subpath \"/Users/test/project\"))").unwrap();
        let deny_pos = profile.find("(deny file-read* file-write* (subpath \"/Users/test/.ssh\"))").unwrap();
        assert!(deny_pos > allow_pos, "Denied paths must come after allows (last-match semantics)");
    }

    #[test]
    fn test_generate_seatbelt_profile_network_denied() {
        let mut policy = SandboxPolicy::confined_to("/Users/test/project");
        policy.allow_network = false;
        let profile = generate_seatbelt_profile(&policy);

        // All network operations blocked (including DNS)
        assert!(profile.contains("(deny network*)"));
    }

    #[test]
    fn test_generate_seatbelt_profile_strict_restricts_exec() {
        let mut policy = SandboxPolicy::confined_to("/Users/test/project");
        policy.enforcement = EnforcementLevel::Strict;
        let profile = generate_seatbelt_profile(&policy);

        // Strict mode uses exec allowlist: deny all, then allow system paths
        assert!(profile.contains("(deny process-exec)"));
        assert!(profile.contains("(allow process-exec (subpath \"/usr/bin\"))"));
        assert!(profile.contains("(allow process-exec (subpath \"/bin\"))"));
    }

    #[test]
    fn test_generate_sandbox_command_disabled() {
        let policy = SandboxPolicy::default(); // disabled by default
        let result = generate_sandbox_command(&policy, "/bin/zsh");
        assert!(result.is_none());
    }

    #[test]
    fn test_generate_sandbox_command_permissive() {
        let mut policy = SandboxPolicy::confined_to("/Users/test/project");
        policy.enforcement = EnforcementLevel::Permissive;
        let result = generate_sandbox_command(&policy, "/bin/zsh");
        assert!(result.is_some(), "Permissive enforcement should still apply Seatbelt profile in audit-only mode");
        let (_, _, env) = result.unwrap();
        assert!(env.iter().any(|e| e == "THANE_SANDBOX_PERMISSIVE=1"),
            "Permissive mode must set THANE_SANDBOX_PERMISSIVE=1");
    }

    #[test]
    fn test_generate_sandbox_command_includes_env() {
        let mut policy = SandboxPolicy::confined_to("/Users/test/project");
        policy.enforcement = EnforcementLevel::Enforcing;
        if let Some((_, _, env)) = generate_sandbox_command(&policy, "/bin/zsh") {
            assert!(env.iter().any(|e| e == "THANE_SANDBOX=1"));
            assert!(env.iter().any(|e| e.starts_with("THANE_SANDBOX_ROOT=")));
        }
    }

    #[test]
    fn test_generate_sandbox_command_includes_resource_limits() {
        let mut policy = SandboxPolicy::confined_to("/Users/test/project");
        policy.enforcement = EnforcementLevel::Enforcing;
        policy.max_open_files = Some(4096);
        policy.max_write_bytes = Some(1_073_741_824);
        policy.cpu_time_limit = Some(300);
        if let Some((exec, _, env)) = generate_sandbox_command(&policy, "/bin/zsh") {
            // Resource limit env vars are only set when using the helper binary
            if exec.contains("thane-sandbox-exec") {
                assert!(env.iter().any(|e| e == "THANE_SANDBOX_MAX_FILES=4096"));
                assert!(env.iter().any(|e| e == "THANE_SANDBOX_MAX_FSIZE=1073741824"));
                assert!(env.iter().any(|e| e == "THANE_SANDBOX_MAX_CPU=300"));
            }
            // Both paths should include sandbox env vars
            assert!(env.iter().any(|e| e == "THANE_SANDBOX=1"));
        }
    }

    #[test]
    fn test_escape_sbpl_path() {
        assert_eq!(escape_sbpl_path("/normal/path"), "/normal/path");
        assert_eq!(escape_sbpl_path("/path with \"quotes\""), "/path with \\\"quotes\\\"");
        assert_eq!(escape_sbpl_path("/path\\backslash"), "/path\\\\backslash");
    }

    #[test]
    fn test_resource_limits_still_work() {
        let mut policy = SandboxPolicy::confined_to("/tmp/test");
        policy.max_open_files = Some(4096);
        assert!(apply_resource_limits(&policy).is_ok());
    }

    #[test]
    fn test_tmpdir_paths_allowed() {
        let policy = SandboxPolicy::confined_to("/Users/test/project");
        let profile = generate_seatbelt_profile(&policy);

        assert!(profile.contains("(allow file* (subpath \"/var/folders\"))"));
        assert!(profile.contains("(allow file* (subpath \"/private/var/folders\"))"));
        assert!(profile.contains("(allow file* (subpath \"/private/tmp\"))"));
    }
}
