use std::collections::HashSet;
#[cfg(target_os = "linux")]
use std::path::Path;

/// Scan for listening TCP ports owned by processes in a given PID group.
///
/// On Linux, reads /proc/net/tcp and /proc/net/tcp6 to find listening sockets,
/// then correlates with /proc/<pid>/fd to find which PIDs own them.
///
/// On macOS, uses `lsof -iTCP -sTCP:LISTEN -nP` to find listening sockets
/// and filters by PID.
#[cfg(target_os = "linux")]
pub fn scan_listening_ports(pids: &[u32]) -> Vec<u16> {
    let pid_set: HashSet<u32> = pids.iter().copied().collect();
    let mut ports = HashSet::new();

    // Parse /proc/net/tcp and /proc/net/tcp6 for listening sockets.
    for tcp_path in &["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(entries) = parse_proc_net_tcp(Path::new(tcp_path)) {
            for entry in entries {
                // State 0A = LISTEN
                if entry.state == 0x0A {
                    // Check if any of our PIDs own this socket's inode.
                    if pid_set.is_empty()
                        || pids
                            .iter()
                            .any(|&pid| pid_owns_inode(pid, entry.inode))
                    {
                        ports.insert(entry.local_port);
                    }
                }
            }
        }
    }

    let mut result: Vec<u16> = ports.into_iter().collect();
    result.sort();
    result
}

/// Scan for listening TCP ports owned by processes in a given PID group using lsof.
#[cfg(target_os = "macos")]
pub fn scan_listening_ports(pids: &[u32]) -> Vec<u16> {
    let pid_set: HashSet<u32> = pids.iter().copied().collect();
    let mut ports = HashSet::new();

    for (pid, port) in lsof_listening_ports() {
        if pid_set.is_empty() || pid.is_some_and(|p| pid_set.contains(&p)) {
            ports.insert(port);
        }
    }

    let mut result: Vec<u16> = ports.into_iter().collect();
    result.sort();
    result
}

/// Scan all listening ports on the system (no PID filter).
#[cfg(target_os = "linux")]
pub fn scan_all_listening_ports() -> Vec<u16> {
    let mut ports = HashSet::new();

    for tcp_path in &["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(entries) = parse_proc_net_tcp(Path::new(tcp_path)) {
            for entry in entries {
                if entry.state == 0x0A {
                    ports.insert(entry.local_port);
                }
            }
        }
    }

    let mut result: Vec<u16> = ports.into_iter().collect();
    result.sort();
    result
}

/// Scan all listening ports on the system (no PID filter) using lsof.
#[cfg(target_os = "macos")]
pub fn scan_all_listening_ports() -> Vec<u16> {
    let mut ports = HashSet::new();

    for (_pid, port) in lsof_listening_ports() {
        ports.insert(port);
    }

    let mut result: Vec<u16> = ports.into_iter().collect();
    result.sort();
    result
}

/// Parse listening TCP ports from lsof output.
///
/// Runs `lsof -iTCP -sTCP:LISTEN -nP -F pn` and returns (Option<pid>, port) pairs.
/// The `-F pn` flag produces machine-parseable output with PID and name fields.
#[cfg(target_os = "macos")]
fn lsof_listening_ports() -> Vec<(Option<u32>, u16)> {
    let output = match std::process::Command::new("lsof")
        .args(["-iTCP", "-sTCP:LISTEN", "-nP", "-F", "pn"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    let mut current_pid: Option<u32> = None;

    for line in stdout.lines() {
        if let Some(pid_str) = line.strip_prefix('p') {
            current_pid = pid_str.parse().ok();
        } else if let Some(name) = line.strip_prefix('n') {
            // Name field looks like "*:8080" or "127.0.0.1:3000" or "[::1]:8080"
            if let Some(port_str) = name.rsplit(':').next() {
                if let Ok(port) = port_str.parse::<u16>() {
                    results.push((current_pid, port));
                }
            }
        }
    }

    results
}

struct TcpEntry {
    local_port: u16,
    state: u8,
    inode: u64,
}

#[cfg(target_os = "linux")]
fn parse_proc_net_tcp(path: &Path) -> Result<Vec<TcpEntry>, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_proc_net_tcp_content(&content))
}

/// Parse /proc/net/tcp content into structured entries.
///
/// This is pure parsing logic, kept available on all platforms for testing.
fn parse_proc_net_tcp_content(content: &str) -> Vec<TcpEntry> {
    let mut entries = Vec::new();

    for line in content.lines().skip(1) {
        // Skip header
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 10 {
            continue;
        }

        // Field 1: local_address (hex_ip:hex_port)
        let local_addr = fields[1];
        let local_port = if let Some(port_hex) = local_addr.split(':').nth(1) {
            u16::from_str_radix(port_hex, 16).unwrap_or(0)
        } else {
            continue;
        };

        // Field 3: state
        let state = u8::from_str_radix(fields[3], 16).unwrap_or(0);

        // Field 9: inode
        let inode = fields[9].parse::<u64>().unwrap_or(0);

        entries.push(TcpEntry {
            local_port,
            state,
            inode,
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    // Realistic /proc/net/tcp content (from a Linux system).
    const PROC_NET_TCP_SAMPLE: &str = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0035 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 12345 1 0000000000000000 100 0 0 10 0
   1: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 12346 1 0000000000000000 100 0 0 10 0
   2: 0100007F:1F90 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 12347 1 0000000000000000 100 0 0 10 0
   3: 00000000:1F91 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 12348 1 0000000000000000 100 0 0 10 0";

    #[test]
    fn test_parse_listen_and_established() {
        let entries = parse_proc_net_tcp_content(PROC_NET_TCP_SAMPLE);
        assert_eq!(entries.len(), 4);

        // Entry 0: 127.0.0.1:53 LISTEN (0x0035 = 53, state 0A)
        assert_eq!(entries[0].local_port, 53);
        assert_eq!(entries[0].state, 0x0A);
        assert_eq!(entries[0].inode, 12345);

        // Entry 1: 0.0.0.0:80 LISTEN (0x0050 = 80)
        assert_eq!(entries[1].local_port, 80);
        assert_eq!(entries[1].state, 0x0A);

        // Entry 2: 127.0.0.1:8080 ESTABLISHED (state 01)
        assert_eq!(entries[2].local_port, 8080);
        assert_eq!(entries[2].state, 0x01);

        // Entry 3: 0.0.0.0:8081 LISTEN
        assert_eq!(entries[3].local_port, 8081);
        assert_eq!(entries[3].state, 0x0A);
    }

    #[test]
    fn test_listen_state_filter() {
        let entries = parse_proc_net_tcp_content(PROC_NET_TCP_SAMPLE);
        let listen_ports: Vec<u16> = entries
            .iter()
            .filter(|e| e.state == 0x0A)
            .map(|e| e.local_port)
            .collect();
        assert_eq!(listen_ports, vec![53, 80, 8081]);
    }

    #[test]
    fn test_malformed_lines() {
        let content = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: short
   1: no_colon 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 999
   2: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 12346 1 0000000000000000 100 0 0 10 0";
        let entries = parse_proc_net_tcp_content(content);
        // Only the last valid line should parse.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].local_port, 80);
    }

    #[test]
    fn test_empty_content() {
        let entries = parse_proc_net_tcp_content("");
        assert!(entries.is_empty());

        // Header only, no data lines.
        let entries = parse_proc_net_tcp_content(
            "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n"
        );
        assert!(entries.is_empty());
    }

    #[test]
    fn test_ipv6_format() {
        // /proc/net/tcp6 uses longer hex addresses but same field layout.
        let content = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 55555 1 0000000000000000 100 0 0 10 0";
        let entries = parse_proc_net_tcp_content(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].local_port, 8080); // 0x1F90
        assert_eq!(entries[0].state, 0x0A);
        assert_eq!(entries[0].inode, 55555);
    }

    #[test]
    fn test_hex_port_conversion() {
        // Verify specific hex → port conversions.
        let content = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:FFFF 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1 1 0000000000000000 100 0 0 10 0
   1: 00000000:0001 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 2 1 0000000000000000 100 0 0 10 0";
        let entries = parse_proc_net_tcp_content(content);
        assert_eq!(entries[0].local_port, 65535); // 0xFFFF
        assert_eq!(entries[1].local_port, 1);     // 0x0001
    }
}

#[cfg(target_os = "linux")]
fn pid_owns_inode(pid: u32, inode: u64) -> bool {
    let fd_dir = format!("/proc/{pid}/fd");
    let entries = match std::fs::read_dir(&fd_dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    for entry in entries.flatten() {
        if let Ok(link) = std::fs::read_link(entry.path()) {
            let link_str = link.to_string_lossy();
            if link_str.contains(&format!("socket:[{inode}]")) {
                return true;
            }
        }
    }

    false
}
