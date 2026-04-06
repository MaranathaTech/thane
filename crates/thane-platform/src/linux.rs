use crate::traits::{DesktopNotifier, NotifyUrgency, PortScanner, ProcessAncestryChecker};

/// Linux implementation of desktop notifications via notify-rust.
pub struct LinuxNotifier;

impl DesktopNotifier for LinuxNotifier {
    fn send_notification(
        &self,
        title: &str,
        body: &str,
        urgency: NotifyUrgency,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let urgency = match urgency {
            NotifyUrgency::Low => notify_rust::Urgency::Low,
            NotifyUrgency::Normal => notify_rust::Urgency::Normal,
            NotifyUrgency::Critical => notify_rust::Urgency::Critical,
        };

        notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .urgency(urgency)
            .appname("thane")
            .show()?;

        Ok(())
    }
}

/// Linux implementation of process ancestry checking via /proc.
pub struct LinuxProcessChecker;

impl ProcessAncestryChecker for LinuxProcessChecker {
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
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        if let Ok(content) = std::fs::read_to_string(&children_path) {
            content
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect()
        } else {
            // Fallback: scan /proc for processes whose PPID matches.
            scan_children_fallback(pid)
        }
    }
}

fn read_ppid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(ppid_str) = line.strip_prefix("PPid:\t") {
            return ppid_str.trim().parse().ok();
        }
    }
    None
}

fn scan_children_fallback(parent_pid: u32) -> Vec<u32> {
    let mut children = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Ok(pid) = name.parse::<u32>()
                && read_ppid(pid) == Some(parent_pid) {
                    children.push(pid);
                }
        }
    }
    children
}

/// Linux implementation of port scanning via /proc/net/tcp.
pub struct LinuxPortScanner;

impl PortScanner for LinuxPortScanner {
    fn scan_listening_ports(&self, pids: &[u32]) -> Vec<u16> {
        thane_core::port_scanner::scan_listening_ports(pids)
    }
}
