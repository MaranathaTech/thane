use gtk4::prelude::*;

/// Widget displaying token usage and estimated cost.
pub struct CostTracker {
    container: gtk4::Box,
    cost_label: gtk4::Label,
    tokens_label: gtk4::Label,
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CostTracker {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        container.set_margin_start(4);
        container.set_margin_end(4);

        let tokens_label = gtk4::Label::new(Some("0 tokens"));
        tokens_label.add_css_class("cost-display");
        container.append(&tokens_label);

        let cost_label = gtk4::Label::new(Some("$0.00"));
        cost_label.add_css_class("cost-display");
        container.append(&cost_label);

        Self {
            container,
            cost_label,
            tokens_label,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    pub fn set_tokens(&self, input_tokens: u64, output_tokens: u64) {
        let total = input_tokens + output_tokens;
        let text = if total >= 1_000_000 {
            format!("{:.1}M tokens", total as f64 / 1_000_000.0)
        } else if total >= 1_000 {
            format!("{:.1}K tokens", total as f64 / 1_000.0)
        } else {
            format!("{total} tokens")
        };
        self.tokens_label.set_text(&text);
    }

    pub fn set_cost(&self, cost_usd: f64) {
        let text = if cost_usd >= 1.0 {
            format!("${cost_usd:.2}")
        } else {
            format!("${cost_usd:.4}")
        };
        self.cost_label.set_text(&text);
    }
}
