use thiserror::Error;

/// Access mode for IPC connections.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Default)]
pub enum AccessMode {
    /// No authentication required (for development).
    Open,
    /// Client must be a descendant process of the thane instance.
    #[default]
    Ancestry,
    /// Client must provide a password/token.
    Token(String),
}


#[derive(Debug, Error)]
pub enum AuthError {
    #[error("access denied: client PID {0} is not a descendant of thane")]
    NotDescendant(u32),
    #[error("access denied: invalid token")]
    InvalidToken,
    #[error("access denied: could not determine client PID")]
    NoPeerCredentials,
}

/// Verify that a connecting client is authorized.
pub fn verify_access(
    mode: &AccessMode,
    client_pid: Option<u32>,
    server_pid: u32,
    provided_token: Option<&str>,
) -> Result<(), AuthError> {
    match mode {
        AccessMode::Open => Ok(()),
        AccessMode::Ancestry => {
            let client_pid = client_pid.ok_or(AuthError::NoPeerCredentials)?;
            if is_descendant(client_pid, server_pid) {
                Ok(())
            } else {
                Err(AuthError::NotDescendant(client_pid))
            }
        }
        AccessMode::Token(expected) => {
            if provided_token == Some(expected.as_str()) {
                Ok(())
            } else {
                Err(AuthError::InvalidToken)
            }
        }
    }
}

/// Check if `child_pid` is a descendant of `ancestor_pid` by walking the process tree.
fn is_descendant(child_pid: u32, ancestor_pid: u32) -> bool {
    let mut current = child_pid;
    loop {
        if current == ancestor_pid {
            return true;
        }
        if current <= 1 {
            return false;
        }
        match read_ppid(current) {
            Some(ppid) => current = ppid,
            None => return false,
        }
    }
}

/// Read the parent PID by parsing /proc/{pid}/status.
#[cfg(target_os = "linux")]
fn read_ppid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(ppid_str) = line.strip_prefix("PPid:\t") {
            return ppid_str.trim().parse().ok();
        }
    }
    None
}

/// Read the parent PID using `ps` on macOS.
#[cfg(target_os = "macos")]
fn read_ppid(pid: u32) -> Option<u32> {
    let output = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse::<u32>().ok()
}

/// Generate a random authentication token (32 hex characters).
pub fn generate_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let s = RandomState::new();
    let mut hasher = s.build_hasher();
    hasher.write_usize(std::process::id() as usize);
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    let h1 = hasher.finish();

    let s2 = RandomState::new();
    let mut hasher2 = s2.build_hasher();
    hasher2.write_u64(h1);
    let h2 = hasher2.finish();

    format!("{h1:016x}{h2:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_mode_allows_all() {
        let mode = AccessMode::Open;
        assert!(verify_access(&mode, None, 1, None).is_ok());
        assert!(verify_access(&mode, Some(999), 1, None).is_ok());
    }

    #[test]
    fn test_token_mode_valid() {
        let mode = AccessMode::Token("secret123".to_string());
        assert!(verify_access(&mode, None, 1, Some("secret123")).is_ok());
    }

    #[test]
    fn test_token_mode_invalid() {
        let mode = AccessMode::Token("secret123".to_string());
        assert!(verify_access(&mode, None, 1, Some("wrong")).is_err());
        assert!(verify_access(&mode, None, 1, None).is_err());
    }

    #[test]
    fn test_ancestry_no_peer_credentials() {
        let mode = AccessMode::Ancestry;
        let result = verify_access(&mode, None, 1, None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AuthError::NoPeerCredentials));
    }

    #[test]
    fn test_ancestry_self_is_descendant() {
        // A process is a "descendant" of itself (pid == ancestor_pid).
        let my_pid = std::process::id();
        let mode = AccessMode::Ancestry;
        assert!(verify_access(&mode, Some(my_pid), my_pid, None).is_ok());
    }

    #[test]
    fn test_ancestry_current_process_descends_from_pid1() {
        // Current process should be a descendant of PID 1 (init/systemd).
        let my_pid = std::process::id();
        let mode = AccessMode::Ancestry;
        assert!(verify_access(&mode, Some(my_pid), 1, None).is_ok());
    }

    #[test]
    fn test_ancestry_unrelated_pid() {
        // PID 1 is NOT a descendant of our process.
        let my_pid = std::process::id();
        let mode = AccessMode::Ancestry;
        assert!(verify_access(&mode, Some(1), my_pid, None).is_err());
    }

    #[test]
    fn test_default_access_mode_is_ancestry() {
        assert_eq!(AccessMode::default(), AccessMode::Ancestry);
    }

    #[test]
    fn test_generate_token_length() {
        let token = generate_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_token_uniqueness() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
    }
}
