use crate::traits::{DesktopNotifier, NotifyUrgency, PortScanner, ProcessAncestryChecker};

/// macOS implementation of desktop notifications via NSUserNotification / UNUserNotificationCenter.
pub struct MacosNotifier;

impl DesktopNotifier for MacosNotifier {
    fn send_notification(
        &self,
        title: &str,
        body: &str,
        _urgency: NotifyUrgency,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Use osascript as a simple cross-version approach.
        // This avoids linking against Objective-C frameworks from Rust directly.
        let script = format!(
            r#"display notification "{}" with title "{}""#,
            body.replace('\\', "\\\\").replace('"', "\\\""),
            title.replace('\\', "\\\\").replace('"', "\\\""),
        );
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        Ok(())
    }
}

/// macOS implementation of process ancestry checking via sysctl KERN_PROC.
pub struct MacosProcessChecker;

impl ProcessAncestryChecker for MacosProcessChecker {
    fn is_descendant(&self, child_pid: u32, ancestor_pid: u32) -> bool {
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

    fn child_pids(&self, pid: u32) -> Vec<u32> {
        // On macOS, enumerate all processes and filter by parent PID.
        scan_children(pid)
    }
}

/// Read parent PID using `ps` on macOS.
fn read_ppid(pid: u32) -> Option<u32> {
    let output = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse::<u32>().ok()
}

/// Scan all processes to find children of the given PID.
fn scan_children(parent_pid: u32) -> Vec<u32> {
    let output = match std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid="])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let s = String::from_utf8_lossy(&output.stdout);
    s.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid: u32 = parts.next()?.parse().ok()?;
            let ppid: u32 = parts.next()?.parse().ok()?;
            if ppid == parent_pid { Some(pid) } else { None }
        })
        .collect()
}

/// macOS implementation of port scanning.
///
/// Delegates to the cross-platform implementation in thane-core which already
/// handles macOS via `lsof`.
pub struct MacosPortScanner;

impl PortScanner for MacosPortScanner {
    fn scan_listening_ports(&self, pids: &[u32]) -> Vec<u16> {
        thane_core::port_scanner::scan_listening_ports(pids)
    }
}
