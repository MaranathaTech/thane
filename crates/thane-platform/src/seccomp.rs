//! seccomp-bpf filter for blocking dangerous syscalls in strict sandbox mode.
//!
//! When the sandbox enforcement level is `Strict`, this module installs a BPF
//! filter that blocks (returns EPERM) dangerous syscalls like `ptrace`, `mount`,
//! `reboot`, `kexec_load`, etc. Normal development operations (file I/O, networking,
//! process management) are still allowed — Landlock handles filesystem restrictions.
//!
//! The filter is applied using `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER)` after
//! `prctl(PR_SET_NO_NEW_PRIVS)` (which Landlock already sets).

use thane_core::sandbox::{EnforcementLevel, SandboxPolicy};

/// Dangerous syscalls to block in strict mode (x86_64 numbers).
///
/// These are syscalls that could be used for privilege escalation,
/// kernel manipulation, or escaping the sandbox.
///
/// Note: `rseq` (334 on x86_64, 293 on aarch64) is intentionally NOT blocked.
/// Modern glibc (2.35+) calls `rseq` during startup for restartable sequences.
/// Blocking it causes `abort()` in any dynamically-linked program (including
/// Node.js / Claude Code). The syscall is safe — it only registers a per-thread
/// critical section descriptor with the kernel for performance optimizations.
#[cfg(target_arch = "x86_64")]
const BLOCKED_SYSCALLS: &[u32] = &[
    101,  // ptrace — process debugging/injection
    165,  // mount — filesystem mount
    166,  // umount2 — filesystem unmount
    169,  // reboot — system reboot
    175,  // init_module — load kernel module
    176,  // delete_module — unload kernel module
    246,  // kexec_load — load new kernel
    304,  // open_by_handle_at — bypass path-based access controls
    310,  // process_vm_readv — read another process's memory
    311,  // process_vm_writev — write another process's memory
    313,  // finit_module — load kernel module from fd
    320,  // kexec_file_load — load new kernel from file
];

/// Network-related syscalls to block when allow_network is false (x86_64).
#[cfg(target_arch = "x86_64")]
const NETWORK_SYSCALLS: &[u32] = &[
    41,   // socket — create network socket
    42,   // connect — connect to remote host
    43,   // accept — accept incoming connection
    44,   // sendto — send data
    45,   // recvfrom — receive data
    46,   // sendmsg — send message
    47,   // recvmsg — receive message
    49,   // bind — bind to address
    50,   // listen — listen for connections
    288,  // accept4 — accept with flags
];

#[cfg(target_arch = "aarch64")]
const BLOCKED_SYSCALLS: &[u32] = &[
    117,  // ptrace
    21,   // mount
    39,   // umount2
    142,  // reboot
    105,  // init_module
    106,  // delete_module
    104,  // kexec_load
    265,  // open_by_handle_at
    270,  // process_vm_readv
    271,  // process_vm_writev
    273,  // finit_module
    294,  // kexec_file_load
];

/// Network-related syscalls to block when allow_network is false (aarch64).
#[cfg(target_arch = "aarch64")]
const NETWORK_SYSCALLS: &[u32] = &[
    198,  // socket
    203,  // connect
    202,  // accept
    206,  // sendto
    207,  // recvfrom
    211,  // sendmsg
    212,  // recvmsg
    200,  // bind
    201,  // listen
    242,  // accept4
];

/// BPF instruction structure for seccomp filters.
#[repr(C)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

/// BPF program structure.
#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

// BPF instruction constants.
const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

// seccomp return values.
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_LOG: u32 = 0x7ffc_0000;

// seccomp constants.
const SECCOMP_MODE_FILTER: libc::c_int = 2;

// Offset of the syscall number in the seccomp_data structure (x86_64/aarch64).
const SECCOMP_DATA_NR_OFFSET: u32 = 0;

/// Error type for seccomp operations.
#[derive(Debug, thiserror::Error)]
pub enum SeccompError {
    #[error("Failed to install seccomp filter: {0}")]
    InstallFilter(std::io::Error),
}

/// Apply a seccomp-bpf filter based on the sandbox policy.
///
/// Installs a BPF filter when:
/// - **Strict mode**: blocks dangerous syscalls + network if disabled
/// - **Enforcing mode + allow_network=false**: blocks network syscalls only
/// - **Permissive mode**: no restrictions
///
/// **IMPORTANT**: Must be called in the forked child process, after
/// `PR_SET_NO_NEW_PRIVS` has been set (Landlock does this).
pub fn apply_seccomp(policy: &SandboxPolicy) -> Result<(), SeccompError> {
    if !policy.enabled || policy.enforcement == EnforcementLevel::Permissive {
        return Ok(());
    }

    // Determine which syscalls to block.
    let needs_dangerous_block = policy.enforcement == EnforcementLevel::Strict;
    let needs_network_block = !policy.allow_network;

    if !needs_dangerous_block && !needs_network_block {
        return Ok(());
    }

    let filter = build_filter(policy);
    install_filter(&filter)
}

/// Build the BPF filter program.
fn build_filter(policy: &SandboxPolicy) -> Vec<SockFilter> {
    // Collect all syscalls to block.
    let mut blocked: Vec<u32> = Vec::new();

    if policy.enforcement == EnforcementLevel::Strict {
        blocked.extend_from_slice(BLOCKED_SYSCALLS);
    }

    if !policy.allow_network {
        blocked.extend_from_slice(NETWORK_SYSCALLS);
    }

    // Deduplicate (shouldn't overlap, but just in case).
    blocked.sort();
    blocked.dedup();

    build_bpf_filter(&blocked, policy.enforcement)
}

/// Build a BPF filter that blocks the given syscall numbers.
fn build_bpf_filter(blocked: &[u32], enforcement: EnforcementLevel) -> Vec<SockFilter> {
    let mut filter = Vec::new();

    // Load the syscall number: LD [seccomp_data.nr]
    filter.push(SockFilter {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: SECCOMP_DATA_NR_OFFSET,
    });

    // For each blocked syscall, add a JEQ check that jumps to DENY.
    // We build:
    //   JEQ #syscall_nr, deny_offset, 0  (if match, jump to deny; else fall through)
    // The deny instruction is at the end, after all JEQ checks.
    let num_blocked = blocked.len();

    for (i, &syscall_nr) in blocked.iter().enumerate() {
        // Jump to deny = (num_blocked - i) instructions ahead.
        // Jump to next check = 0 (fall through).
        let jt = (num_blocked - i) as u8;
        filter.push(SockFilter {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt,
            jf: 0,
            k: syscall_nr,
        });
    }

    // Default action: ALLOW all non-blocked syscalls.
    filter.push(SockFilter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    // Deny action: return EPERM (or LOG in permissive).
    let deny_action = if enforcement == EnforcementLevel::Permissive {
        SECCOMP_RET_LOG
    } else {
        SECCOMP_RET_ERRNO | (libc::EPERM as u32 & 0xFFFF)
    };

    filter.push(SockFilter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: deny_action,
    });

    filter
}

/// Install the BPF filter using prctl.
fn install_filter(filter: &[SockFilter]) -> Result<(), SeccompError> {
    let prog = SockFprog {
        len: filter.len() as u16,
        filter: filter.as_ptr(),
    };

    let ret = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            SECCOMP_MODE_FILTER,
            &prog as *const SockFprog,
        )
    };

    if ret != 0 {
        return Err(SeccompError::InstallFilter(std::io::Error::last_os_error()));
    }

    Ok(())
}

/// Check if seccomp is supported on this kernel.
pub fn is_seccomp_supported() -> bool {
    // Try to read /proc/sys/kernel/seccomp (or check prctl).
    // A simple approach: check if PR_GET_SECCOMP works.
    let ret = unsafe { libc::prctl(libc::PR_GET_SECCOMP) };
    // ret == 0 means seccomp is not active (but supported).
    // ret == 2 means filter mode is active.
    // ret == -1 with EINVAL means not supported.
    ret >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seccomp_supported_check() {
        // Just verify it doesn't panic.
        let _supported = is_seccomp_supported();
    }

    #[test]
    fn test_disabled_policy_is_noop() {
        let policy = SandboxPolicy::default();
        assert!(apply_seccomp(&policy).is_ok());
    }

    #[test]
    fn test_enforcing_mode_with_network_allowed_skips_seccomp() {
        let policy = SandboxPolicy::confined_to("/home/user/project");
        // Default: enforcement=Enforcing, allow_network=true → no seccomp needed.
        assert_eq!(policy.enforcement, EnforcementLevel::Enforcing);
        assert!(policy.allow_network);
        assert!(apply_seccomp(&policy).is_ok());
    }

    #[test]
    fn test_build_filter_strict_mode() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.enforcement = EnforcementLevel::Strict;

        let filter = build_filter(&policy);
        // Should have: 1 (load) + N (JEQ checks) + 1 (allow) + 1 (deny)
        assert_eq!(filter.len(), 1 + BLOCKED_SYSCALLS.len() + 2);

        // First instruction should be LD ABS.
        assert_eq!(filter[0].code, BPF_LD | BPF_W | BPF_ABS);

        // Last two should be RET.
        let allow = &filter[filter.len() - 2];
        assert_eq!(allow.code, BPF_RET | BPF_K);
        assert_eq!(allow.k, SECCOMP_RET_ALLOW);

        let deny = &filter[filter.len() - 1];
        assert_eq!(deny.code, BPF_RET | BPF_K);
        assert_eq!(deny.k & 0xFFFF_0000, SECCOMP_RET_ERRNO);
    }

    #[test]
    fn test_build_filter_network_blocked() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.allow_network = false;
        // Enforcing + no network → filter blocks only network syscalls.

        let filter = build_filter(&policy);
        assert_eq!(filter.len(), 1 + NETWORK_SYSCALLS.len() + 2);
    }

    #[test]
    fn test_build_filter_strict_and_no_network() {
        let mut policy = SandboxPolicy::confined_to("/home/user/project");
        policy.enforcement = EnforcementLevel::Strict;
        policy.allow_network = false;

        let filter = build_filter(&policy);
        // Both dangerous + network syscalls blocked (deduplicated).
        let mut expected: Vec<u32> = Vec::new();
        expected.extend_from_slice(BLOCKED_SYSCALLS);
        expected.extend_from_slice(NETWORK_SYSCALLS);
        expected.sort();
        expected.dedup();
        assert_eq!(filter.len(), 1 + expected.len() + 2);
    }

    #[test]
    fn test_blocked_syscalls_not_empty() {
        assert!(!BLOCKED_SYSCALLS.is_empty());
    }

    #[test]
    fn test_network_syscalls_not_empty() {
        assert!(!NETWORK_SYSCALLS.is_empty());
    }

    #[test]
    fn test_rseq_not_blocked() {
        // rseq must NOT be in the blocked list — modern glibc (2.35+) calls it
        // on startup and blocking it causes abort() in any dynamically-linked
        // program (including Node.js / Claude Code).
        #[cfg(target_arch = "x86_64")]
        {
            let rseq_nr: u32 = 334;
            assert!(
                !BLOCKED_SYSCALLS.contains(&rseq_nr),
                "rseq (syscall {rseq_nr}) must not be blocked — it crashes glibc 2.35+"
            );
        }
        #[cfg(target_arch = "aarch64")]
        {
            let rseq_nr: u32 = 293;
            assert!(
                !BLOCKED_SYSCALLS.contains(&rseq_nr),
                "rseq (syscall {rseq_nr}) must not be blocked — it crashes glibc 2.35+"
            );
        }
    }
}
