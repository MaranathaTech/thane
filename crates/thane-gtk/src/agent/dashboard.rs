use thane_core::sidebar::AgentStatus;
use gtk4::prelude::*;

/// Widget showing agent activity overview.
pub struct AgentDashboard {
    container: gtk4::Box,
    status_label: gtk4::Label,
    spinner: gtk4::Spinner,
}

impl Default for AgentDashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentDashboard {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        container.set_margin_start(4);
        container.set_margin_end(4);

        let spinner = gtk4::Spinner::new();
        spinner.set_visible(false);
        container.append(&spinner);

        let status_label = gtk4::Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        container.append(&status_label);

        Self {
            container,
            status_label,
            spinner,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    pub fn set_status(&self, status: &AgentStatus, agent_names: &[String]) {
        match status {
            AgentStatus::Inactive => {
                self.status_label.set_text("Agent: idle");
                self.status_label.remove_css_class("agent-active");
                self.status_label.remove_css_class("agent-stalled");
                self.status_label.add_css_class("agent-inactive");
                self.spinner.set_visible(false);
                self.spinner.stop();
            }
            AgentStatus::Active => {
                let label = format_agent_label(agent_names);
                self.status_label.set_text(&label);
                self.status_label.remove_css_class("agent-inactive");
                self.status_label.remove_css_class("agent-stalled");
                self.status_label.add_css_class("agent-active");
                self.spinner.set_visible(true);
                self.spinner.start();
            }
            AgentStatus::Stalled => {
                let label = if agent_names.is_empty() {
                    "Agent stalled".to_string()
                } else {
                    let names = agent_names.join(", ");
                    format!("Agent stalled: {names}")
                };
                self.status_label.set_text(&label);
                self.status_label.remove_css_class("agent-inactive");
                self.status_label.remove_css_class("agent-active");
                self.status_label.add_css_class("agent-stalled");
                self.spinner.set_visible(false);
                self.spinner.stop();
            }
        }
    }
}

/// Format the agent label like macOS: "Agent: claude" for one agent,
/// "Agents: claude, codex" for multiple.
fn format_agent_label(agent_names: &[String]) -> String {
    match agent_names.len() {
        0 => "Agent running".to_string(),
        1 => format!("Agent: {}", agent_names[0]),
        _ => {
            let names = agent_names.join(", ");
            format!("Agents: {names}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_agent_label_no_names() {
        assert_eq!(format_agent_label(&[]), "Agent running");
    }

    #[test]
    fn test_format_agent_label_single() {
        let names = vec!["claude".to_string()];
        assert_eq!(format_agent_label(&names), "Agent: claude");
    }

    #[test]
    fn test_format_agent_label_multiple() {
        let names = vec!["claude".to_string(), "codex".to_string()];
        assert_eq!(format_agent_label(&names), "Agents: claude, codex");
    }

    #[test]
    fn test_format_agent_label_three() {
        let names = vec![
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
        ];
        assert_eq!(format_agent_label(&names), "Agents: claude, codex, gemini");
    }
}
