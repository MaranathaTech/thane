/// Parsed OSC (Operating System Command) sequence.
#[derive(Debug, Clone, PartialEq)]
pub enum OscEvent {
    /// OSC 0: Set window title.
    SetTitle(String),
    /// OSC 7: Set current working directory.
    SetCwd(String),
    /// OSC 9: iTerm2-style notification.
    Notification { body: String },
    /// OSC 99: Kitty notification protocol.
    KittyNotification { payload: String },
    /// OSC 777: rxvt-unicode notification.
    RxvtNotification { title: String, body: String },
    /// OSC 133: Shell integration mark.
    ShellIntegration(ShellMark),
    /// Unrecognized OSC.
    Unknown { number: u32, payload: String },
}

/// Shell integration marks (OSC 133).
#[derive(Debug, Clone, PartialEq)]
pub enum ShellMark {
    /// Prompt start (A)
    PromptStart,
    /// Command start (B)
    CommandStart,
    /// Command executed (C)
    CommandExecuted,
    /// Command finished (D) with optional exit code
    CommandFinished(Option<i32>),
}

/// Parse an OSC sequence number and payload into a structured event.
pub fn parse_osc(number: u32, payload: &str) -> OscEvent {
    match number {
        0 | 2 => OscEvent::SetTitle(payload.to_string()),
        7 => OscEvent::SetCwd(parse_osc7_uri(payload)),
        9 => OscEvent::Notification {
            body: payload.to_string(),
        },
        99 => OscEvent::KittyNotification {
            payload: payload.to_string(),
        },
        133 => parse_osc133(payload),
        777 => {
            let parts: Vec<&str> = payload.splitn(3, ';').collect();
            if parts.len() >= 3 && parts[0] == "notify" {
                OscEvent::RxvtNotification {
                    title: parts[1].to_string(),
                    body: parts[2].to_string(),
                }
            } else {
                OscEvent::Unknown {
                    number,
                    payload: payload.to_string(),
                }
            }
        }
        _ => OscEvent::Unknown {
            number,
            payload: payload.to_string(),
        },
    }
}

/// Parse OSC 7 URI: `file://hostname/path`
fn parse_osc7_uri(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("file://") {
        // Skip hostname (everything up to the next '/')
        if let Some(slash_pos) = rest.find('/') {
            return rest[slash_pos..].to_string();
        }
    }
    uri.to_string()
}

/// Parse OSC 133 shell integration marks.
fn parse_osc133(payload: &str) -> OscEvent {
    match payload.chars().next() {
        Some('A') => OscEvent::ShellIntegration(ShellMark::PromptStart),
        Some('B') => OscEvent::ShellIntegration(ShellMark::CommandStart),
        Some('C') => OscEvent::ShellIntegration(ShellMark::CommandExecuted),
        Some('D') => {
            let exit_code = payload
                .get(2..)
                .and_then(|s| s.trim_start_matches(';').parse().ok());
            OscEvent::ShellIntegration(ShellMark::CommandFinished(exit_code))
        }
        _ => OscEvent::Unknown {
            number: 133,
            payload: payload.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_title() {
        assert_eq!(
            parse_osc(0, "my terminal"),
            OscEvent::SetTitle("my terminal".to_string())
        );
    }

    #[test]
    fn test_parse_cwd() {
        assert_eq!(
            parse_osc(7, "file://localhost/home/user/project"),
            OscEvent::SetCwd("/home/user/project".to_string())
        );
    }

    #[test]
    fn test_parse_notification() {
        assert_eq!(
            parse_osc(9, "Build done!"),
            OscEvent::Notification {
                body: "Build done!".to_string()
            }
        );
    }

    #[test]
    fn test_parse_shell_marks() {
        assert_eq!(
            parse_osc(133, "A"),
            OscEvent::ShellIntegration(ShellMark::PromptStart)
        );
        assert_eq!(
            parse_osc(133, "D;0"),
            OscEvent::ShellIntegration(ShellMark::CommandFinished(Some(0)))
        );
    }

    #[test]
    fn test_parse_osc777() {
        assert_eq!(
            parse_osc(777, "notify;Build;Tests passed"),
            OscEvent::RxvtNotification {
                title: "Build".to_string(),
                body: "Tests passed".to_string()
            }
        );
    }
}
