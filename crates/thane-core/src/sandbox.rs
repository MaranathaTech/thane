use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Sandbox policy for a workspace's terminal sessions.
///
/// When enabled, the agent is restricted to a set of allowed directories.
/// This is enforced at multiple levels:
/// 1. **Landlock LSM** (Linux 5.13+) — kernel-level filesystem restriction
/// 2. **seccomp-bpf** — block dangerous syscalls (optional strict mode)
/// 3. **Namespace isolation** — mount namespace to hide paths (optional)
/// 4. **Audit integration** — log any sandbox violation attempts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Whether sandboxing is enabled for this workspace.
    pub enabled: bool,
    /// The root directory the agent is confined to (typically the workspace CWD).
    pub root_dir: PathBuf,
    /// Additional directories the agent is allowed to read (e.g. /usr, /lib for toolchains).
    pub read_only_paths: Vec<PathBuf>,
    /// Additional directories the agent is allowed to read and write.
    pub read_write_paths: Vec<PathBuf>,
    /// Paths explicitly denied even if they're under root_dir (e.g. .env, .ssh).
    pub denied_paths: Vec<PathBuf>,
    /// Whether to allow network access.
    pub allow_network: bool,
    /// Maximum number of open file descriptors.
    pub max_open_files: Option<u64>,
    /// Maximum writable bytes (to prevent disk-filling attacks).
    pub max_write_bytes: Option<u64>,
    /// CPU time limit in seconds (RLIMIT_CPU — sends SIGXCPU then SIGKILL).
    pub cpu_time_limit: Option<u64>,
    /// Enforcement level.
    pub enforcement: EnforcementLevel,
}

/// How strictly to enforce sandbox violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementLevel {
    /// Log violations but don't block (audit-only mode).
    Permissive,
    /// Block violations and log them.
    Enforcing,
    /// Block violations, log them, and terminate the offending process.
    Strict,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            root_dir: PathBuf::from("."),
            read_only_paths: default_read_only_paths(),
            read_write_paths: default_read_write_paths(),
            denied_paths: default_denied_paths(),
            allow_network: true,
            max_open_files: None,
            max_write_bytes: None,
            cpu_time_limit: None,
            enforcement: EnforcementLevel::Enforcing,
        }
    }
}

impl SandboxPolicy {
    /// Create a sandbox policy confining the agent to the given directory.
    pub fn confined_to(root_dir: impl Into<PathBuf>) -> Self {
        let root_dir = root_dir.into();
        let mut rw = default_read_write_paths();
        rw.push(root_dir.clone());
        Self {
            enabled: true,
            root_dir,
            read_write_paths: rw,
            ..Self::default()
        }
    }

    /// Check if a given path is allowed for reading.
    pub fn can_read(&self, path: &Path) -> bool {
        if !self.enabled {
            return true;
        }

        let path = normalize_path(path);

        // Check denied paths first.
        if self.is_denied(&path) {
            return false;
        }

        // Check if under root dir.
        if path.starts_with(&self.root_dir) {
            return true;
        }

        // Check read-only paths.
        if self.read_only_paths.iter().any(|p| path.starts_with(p)) {
            return true;
        }

        // Check read-write paths.
        if self.read_write_paths.iter().any(|p| path.starts_with(p)) {
            return true;
        }

        false
    }

    /// Check if a given path is allowed for writing.
    pub fn can_write(&self, path: &Path) -> bool {
        if !self.enabled {
            return true;
        }

        let path = normalize_path(path);

        // Check denied paths first.
        if self.is_denied(&path) {
            return false;
        }

        // Check if under root dir.
        if path.starts_with(&self.root_dir) {
            return true;
        }

        // Check read-write paths (read-only paths are not writable).
        if self.read_write_paths.iter().any(|p| path.starts_with(p)) {
            return true;
        }

        false
    }

    /// Check if a path is in the denied list.
    fn is_denied(&self, path: &Path) -> bool {
        self.denied_paths.iter().any(|denied| {
            path.starts_with(denied) || path == denied
        })
    }

    /// Generate the Landlock ruleset specification for this policy.
    ///
    /// Returns a list of (path, access_flags) tuples suitable for
    /// constructing a Landlock ruleset.
    pub fn landlock_rules(&self) -> Vec<LandlockRule> {
        if !self.enabled {
            return Vec::new();
        }

        let mut rules = Vec::new();

        // Root directory gets full read-write access.
        rules.push(LandlockRule {
            path: self.root_dir.clone(),
            access: LandlockAccess::ReadWrite,
        });

        // Read-only paths.
        for path in &self.read_only_paths {
            rules.push(LandlockRule {
                path: path.clone(),
                access: LandlockAccess::ReadOnly,
            });
        }

        // Additional read-write paths.
        for path in &self.read_write_paths {
            rules.push(LandlockRule {
                path: path.clone(),
                access: LandlockAccess::ReadWrite,
            });
        }

        rules
    }

    /// Get the resource limits to apply in the child process.
    ///
    /// Returns a list of (resource_id, soft_limit, hard_limit) tuples
    /// corresponding to POSIX rlimit resources.
    pub fn resource_limits(&self) -> Vec<ResourceLimit> {
        if !self.enabled {
            return Vec::new();
        }

        let mut limits = Vec::new();

        if let Some(max_fds) = self.max_open_files {
            limits.push(ResourceLimit {
                resource: RlimitResource::NoFile,
                soft: max_fds,
                hard: max_fds,
            });
        }

        if let Some(max_bytes) = self.max_write_bytes {
            limits.push(ResourceLimit {
                resource: RlimitResource::FSize,
                soft: max_bytes,
                hard: max_bytes,
            });
        }

        if let Some(cpu_secs) = self.cpu_time_limit {
            // Soft limit triggers SIGXCPU, hard limit triggers SIGKILL.
            // Give a 10-second grace period between soft and hard.
            limits.push(ResourceLimit {
                resource: RlimitResource::Cpu,
                soft: cpu_secs,
                hard: cpu_secs + 10,
            });
        }

        limits
    }

    /// Generate environment variables that enforce the sandbox in child shells.
    /// These set up a restricted PATH and HOME.
    pub fn env_vars(&self) -> Vec<(String, String)> {
        if !self.enabled {
            return Vec::new();
        }

        let mut vars = Vec::new();
        vars.push((
            "THANE_SANDBOX".to_string(),
            "1".to_string(),
        ));
        vars.push((
            "THANE_SANDBOX_ROOT".to_string(),
            self.root_dir.to_string_lossy().to_string(),
        ));
        if self.enforcement == EnforcementLevel::Strict {
            vars.push((
                "THANE_SANDBOX_STRICT".to_string(),
                "1".to_string(),
            ));
        }
        vars
    }
}

/// A resource limit to apply via setrlimit(2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimit {
    pub resource: RlimitResource,
    pub soft: u64,
    pub hard: u64,
}

/// POSIX resource limit identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RlimitResource {
    /// RLIMIT_NOFILE: max number of open file descriptors.
    NoFile,
    /// RLIMIT_FSIZE: max file size in bytes a process can create.
    FSize,
    /// RLIMIT_CPU: max CPU time in seconds.
    Cpu,
}

/// A Landlock filesystem access rule.
#[derive(Debug, Clone)]
pub struct LandlockRule {
    pub path: PathBuf,
    pub access: LandlockAccess,
}

/// Landlock access levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandlockAccess {
    ReadOnly,
    ReadWrite,
}

/// Default system paths that agents should be able to read (toolchains, libraries).
fn default_read_only_paths() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/nobody"));

    let mut paths = vec![
        PathBuf::from("/usr"),
        PathBuf::from("/bin"),
        PathBuf::from("/sbin"),
        // /etc is needed for shell startup (/etc/profile, /etc/bash.bashrc, /etc/environment,
        // /etc/passwd, /etc/nsswitch.conf, /etc/hosts, /etc/resolv.conf, etc).
        PathBuf::from("/etc"),
    ];

    // Platform-specific system paths.
    #[cfg(target_os = "linux")]
    {
        paths.push(PathBuf::from("/lib"));
        paths.push(PathBuf::from("/lib64"));
        // /proc is needed for process introspection (ps, /proc/self, etc).
        paths.push(PathBuf::from("/proc"));
        // /sys is needed for cgroup introspection — Node.js reads
        // /sys/fs/cgroup/.../memory.max on startup and aborts if blocked.
        paths.push(PathBuf::from("/sys"));
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("/usr/local"));
        paths.push(PathBuf::from("/opt/homebrew"));
        paths.push(PathBuf::from("/Applications"));
    }

    // Shell dotfiles — bash/zsh/fish need these to start without errors.
    paths.extend([
        home.join(".bashrc"),
        home.join(".bash_profile"),
        home.join(".bash_logout"),
        home.join(".profile"),
        home.join(".zshrc"),
        home.join(".zshenv"),
        home.join(".zprofile"),
        home.join(".config/fish"),
        // Rust toolchain
        home.join(".cargo"),
        home.join(".rustup"),
        // Node.js
        home.join(".nvm"),
        // User-installed tools (e.g. ~/.local/bin/ for pipx, npm global, etc.)
        home.join(".local"),
    ]);

    // Runtime state — D-Bus sockets, systemd user services, etc.
    #[cfg(target_os = "linux")]
    paths.push(PathBuf::from("/run"));

    paths
}

/// Default paths that need read-write access for basic terminal/shell operation.
fn default_read_write_paths() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/nobody"));

    let mut paths = vec![
        // /dev is needed for /dev/null, /dev/tty, /dev/pts/*, /dev/urandom, etc.
        PathBuf::from("/dev"),
        // /tmp often needs write access for build tools, sockets, temp files.
        PathBuf::from("/tmp"),
    ];

    #[cfg(target_os = "macos")]
    paths.push(PathBuf::from("/private/tmp"));

    // Claude Code needs RW access for session data, history, debug logs.
    // Note: ~/.claude/.credentials.json is in the denied list so it stays protected.
    paths.push(home.join(".claude"));
    // Claude Code settings directory.
    paths.push(home.join(".config/claude"));
    // npm cache — needed by Claude Code (Node.js).
    paths.push(home.join(".npm"));

    paths
}

/// Default paths that should be denied even under the root directory.
fn default_denied_paths() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/nobody"));
    #[allow(unused_mut)]
    let mut paths = vec![
        // SSH & GPG keys
        home.join(".ssh"),
        home.join(".gnupg"),
        home.join(".pgpass"),
        // Cloud provider credentials
        home.join(".aws"),
        home.join(".config/gcloud"),
        home.join(".azure"),
        home.join(".docker"),
        home.join(".kube"),
        // Package manager auth tokens
        home.join(".npmrc"),
        home.join(".pypirc"),
        home.join(".gem/credentials"),
        home.join(".config/pip"),
        // JVM ecosystem credentials
        home.join(".m2/settings.xml"),
        home.join(".m2/settings-security.xml"),
        home.join(".gradle/gradle.properties"),
        home.join(".ivy2/.credentials"),
        home.join(".sbt/credentials"),
        // Ruby
        home.join(".gem/credentials"),
        // Environment files with secrets
        home.join(".env"),
        home.join(".env.local"),
        home.join(".env.production"),
        // Network auth
        home.join(".netrc"),
        // Git credentials
        home.join(".git-credentials"),
        // GitHub CLI tokens
        home.join(".config/gh"),
        // Terraform credentials
        home.join(".terraform.d/credentials.tfrc.json"),
        // 1Password CLI
        home.join(".config/op"),
        // Claude Code credentials
        home.join(".claude/.credentials.json"),
    ];

    // macOS-specific sensitive paths
    #[cfg(target_os = "macos")]
    {
        paths.push(home.join("Library/Keychains"));
        paths.push(home.join("Library/Cookies"));
        // macOS HTTP cookies database
        paths.push(PathBuf::from("/private/var/db/com.apple.curl.http-cookies"));
    }

    paths
}

/// Normalize a path by resolving symlinks, `.` and `..`.
///
/// Always resolves symlinks to prevent symlink-based sandbox escapes
/// (e.g., `~/project/data -> ~/.ssh` bypassing denied paths).
/// If the path doesn't exist yet, we resolve what we can (parent dir)
/// and append the remaining component.
fn normalize_path(path: &Path) -> PathBuf {
    // Try full canonicalization first (resolves all symlinks).
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    // Path doesn't exist yet — resolve the parent to catch symlinks there.
    if let Some(parent) = path.parent() {
        if let Ok(canonical_parent) = parent.canonicalize() {
            if let Some(file_name) = path.file_name() {
                return canonical_parent.join(file_name);
            }
        }
    }
    // Last resort: return as-is (should be rare).
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_sandbox_allows_everything() {
        let policy = SandboxPolicy::default();
        assert!(policy.can_read(Path::new("/etc/passwd")));
        assert!(policy.can_write(Path::new("/tmp/test")));
    }

    #[test]
    fn test_confined_sandbox() {
        let policy = SandboxPolicy::confined_to("/home/user/project");

        // Can read/write within the project.
        assert!(policy.can_read(Path::new("/home/user/project/src/main.rs")));
        assert!(policy.can_write(Path::new("/home/user/project/target/debug/app")));

        // Can read system paths.
        assert!(policy.can_read(Path::new("/usr/bin/git")));
        #[cfg(target_os = "linux")]
        assert!(policy.can_read(Path::new("/lib/x86_64-linux-gnu/libc.so.6")));
        #[cfg(target_os = "macos")]
        assert!(policy.can_read(Path::new("/opt/homebrew/bin/git")));

        // Cannot write to system paths.
        assert!(!policy.can_write(Path::new("/usr/bin/malicious")));
        assert!(!policy.can_write(Path::new("/etc/passwd")));

        // Cannot read outside allowed paths.
        assert!(!policy.can_read(Path::new("/home/user/other-project/secrets")));
    }

    #[test]
    fn test_denied_paths() {
        let mut policy = SandboxPolicy::confined_to("/home/user");
        // Even though /home/user is the root, .ssh should be denied.
        let home = PathBuf::from("/home/user");
        policy.denied_paths = vec![home.join(".ssh"), home.join(".env")];

        assert!(!policy.can_read(Path::new("/home/user/.ssh/id_rsa")));
        assert!(!policy.can_write(Path::new("/home/user/.ssh/id_rsa")));
        assert!(!policy.can_read(Path::new("/home/user/.env")));

        // But regular project files are fine.
        assert!(policy.can_read(Path::new("/home/user/project/main.rs")));
    }

    #[test]
    fn test_landlock_rules() {
        let policy = SandboxPolicy::confined_to("/home/user/project");
        let rules = policy.landlock_rules();

        assert!(!rules.is_empty());
        // Root dir should have read-write access.
        assert!(rules.iter().any(|r| r.path == PathBuf::from("/home/user/project")
            && r.access == LandlockAccess::ReadWrite));
        // System paths should be read-only.
        assert!(rules
            .iter()
            .any(|r| r.path == PathBuf::from("/usr") && r.access == LandlockAccess::ReadOnly));
    }

    #[test]
    fn test_env_vars() {
        let policy = SandboxPolicy::confined_to("/home/user/project");
        let vars = policy.env_vars();

        assert!(vars.iter().any(|(k, v)| k == "THANE_SANDBOX" && v == "1"));
        assert!(vars
            .iter()
            .any(|(k, _)| k == "THANE_SANDBOX_ROOT"));
    }

    #[test]
    fn test_resource_limits_none_by_default() {
        let policy = SandboxPolicy::confined_to("/home/user/project");
        assert!(policy.resource_limits().is_empty());
    }

    #[test]
    fn test_resource_limits_nofile() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.max_open_files = Some(2048);

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].resource, RlimitResource::NoFile);
        assert_eq!(limits[0].soft, 2048);
        assert_eq!(limits[0].hard, 2048);
    }

    #[test]
    fn test_resource_limits_fsize() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.max_write_bytes = Some(1024 * 1024 * 100);

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].resource, RlimitResource::FSize);
        assert_eq!(limits[0].soft, 1024 * 1024 * 100);
    }

    #[test]
    fn test_resource_limits_cpu_with_grace_period() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.cpu_time_limit = Some(120);

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 1);
        assert_eq!(limits[0].resource, RlimitResource::Cpu);
        assert_eq!(limits[0].soft, 120);
        assert_eq!(limits[0].hard, 130); // 10s grace period
    }

    #[test]
    fn test_resource_limits_disabled_sandbox() {
        let mut policy = SandboxPolicy::default();
        policy.max_open_files = Some(1024);
        // When sandbox is disabled, resource_limits returns empty.
        assert!(policy.resource_limits().is_empty());
    }

    #[test]
    fn test_resource_limits_multiple() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.max_open_files = Some(1024);
        policy.max_write_bytes = Some(50_000_000);
        policy.cpu_time_limit = Some(60);

        let limits = policy.resource_limits();
        assert_eq!(limits.len(), 3);
        assert!(limits.iter().any(|l| l.resource == RlimitResource::NoFile));
        assert!(limits.iter().any(|l| l.resource == RlimitResource::FSize));
        assert!(limits.iter().any(|l| l.resource == RlimitResource::Cpu));
    }

    #[test]
    fn test_claude_code_paths_allowed() {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/nobody"));
        let rw_paths = default_read_write_paths();
        let ro_paths = default_read_only_paths();

        // Claude Code RW paths must be in default_read_write_paths.
        assert!(
            rw_paths.contains(&home.join(".claude")),
            "~/.claude must be in default RW paths for Claude Code session data"
        );
        assert!(
            rw_paths.contains(&home.join(".config/claude")),
            "~/.config/claude must be in default RW paths for Claude settings"
        );
        assert!(
            rw_paths.contains(&home.join(".npm")),
            "~/.npm must be in default RW paths for npm cache"
        );

        // ~/.local must be in read-only paths for user-installed tools.
        assert!(
            ro_paths.contains(&home.join(".local")),
            "~/.local must be in default RO paths for user-installed tools"
        );

        // /sys must be in read-only paths — Node.js reads cgroup info on startup.
        #[cfg(target_os = "linux")]
        assert!(
            ro_paths.contains(&PathBuf::from("/sys")),
            "/sys must be in default RO paths for cgroup introspection"
        );

        // ~/.claude/.credentials.json must still be denied.
        let denied = default_denied_paths();
        assert!(
            denied.contains(&home.join(".claude/.credentials.json")),
            "~/.claude/.credentials.json must remain in denied paths"
        );
    }
}
