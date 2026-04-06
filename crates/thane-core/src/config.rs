use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::CoreError;

/// Parsed configuration for thane.
///
/// Reads Ghostty-format config files (key = value) and provides
/// thane-specific overrides.
#[derive(Debug, Clone)]
pub struct Config {
    /// Raw key-value pairs from the config file.
    values: HashMap<String, String>,
    /// Path the config was loaded from, if any.
    pub source_path: Option<PathBuf>,
    /// Keybinding entries (multiple values allowed for the `keybind` key).
    keybind_entries: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut values = HashMap::new();
        // Sensible defaults
        values.insert("font-family".to_string(), "JetBrains Mono NL Light".to_string());
        values.insert("font-size".to_string(), "13".to_string());
        values.insert("scrollback-limit".to_string(), "10000".to_string());
        values.insert("cursor-style".to_string(), "block".to_string());
        values.insert("cursor-style-blink".to_string(), "true".to_string());
        values.insert("window-padding-x".to_string(), "2".to_string());
        values.insert("window-padding-y".to_string(), "2".to_string());
        values.insert("confirm-close-surface".to_string(), "true".to_string());
        values.insert(
            "shell-integration".to_string(),
            "detect".to_string(),
        );
        values.insert(
            "terminal-foreground".to_string(),
            "#e4e4e7".to_string(),
        );
        Self {
            values,
            source_path: None,
            keybind_entries: Vec::new(),
        }
    }
}

impl Config {
    /// Load config from a Ghostty-format file.
    ///
    /// Format: `key = value` lines, `#` comments, blank lines ignored.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        let content = std::fs::read_to_string(path)?;
        let mut config = Config {
            source_path: Some(path.to_path_buf()),
            ..Self::default()
        };
        config.parse_content(&content)?;
        Ok(config)
    }

    /// Load from default locations (XDG_CONFIG_HOME/ghostty/config, then
    /// XDG_CONFIG_HOME/thane/config).
    pub fn load_default() -> Self {
        let mut config = Self::default();

        // Try Ghostty config first
        if let Some(config_dir) = dirs::config_dir() {
            let ghostty_config = config_dir.join("ghostty").join("config");
            if ghostty_config.exists()
                && let Ok(content) = std::fs::read_to_string(&ghostty_config) {
                    let _ = config.parse_content(&content);
                    config.source_path = Some(ghostty_config);
                }

            // Override with thane-specific config
            let thane_config = config_dir.join("thane").join("config");
            if thane_config.exists()
                && let Ok(content) = std::fs::read_to_string(&thane_config) {
                    let _ = config.parse_content(&content);
                    config.source_path = Some(thane_config);
                }
        }

        config
    }

    fn parse_content(&mut self, content: &str) -> Result<(), CoreError> {
        for line in content.lines() {
            let line = line.trim();

            // Skip comments and blank lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim().to_string();
                if key == "keybind" {
                    self.keybind_entries.push(value);
                } else {
                    self.values.insert(key, value);
                }
            }
        }
        Ok(())
    }

    /// Get a config value as a string.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    /// Get a config value parsed as the given type.
    pub fn get_parsed<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.values.get(key).and_then(|v| v.parse().ok())
    }

    /// Get a config value or a default.
    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.values
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    // Convenience accessors for common config values

    pub fn font_family(&self) -> &str {
        self.values
            .get("font-family")
            .map(|s| s.as_str())
            .unwrap_or("JetBrains Mono NL Light")
    }

    pub fn font_size(&self) -> f64 {
        self.get_parsed("font-size").unwrap_or(13.0)
    }

    pub fn terminal_font_color(&self) -> &str {
        self.values
            .get("terminal-foreground")
            .map(|s| s.as_str())
            .unwrap_or("#e4e4e7")
    }

    pub fn scrollback_limit(&self) -> i64 {
        self.get_parsed("scrollback-limit").unwrap_or(10000)
    }

    pub fn cursor_style(&self) -> &str {
        self.values
            .get("cursor-style")
            .map(|s| s.as_str())
            .unwrap_or("block")
    }

    pub fn cursor_blink(&self) -> bool {
        self.get_parsed("cursor-style-blink").unwrap_or(true)
    }

    pub fn confirm_close_surface(&self) -> bool {
        self.get_parsed("confirm-close-surface").unwrap_or(true)
    }

    pub fn window_padding_x(&self) -> i32 {
        self.get_parsed("window-padding-x").unwrap_or(2)
    }

    pub fn window_padding_y(&self) -> i32 {
        self.get_parsed("window-padding-y").unwrap_or(2)
    }

    pub fn ui_text_size(&self) -> f64 {
        self.get_parsed("ui-text-size").unwrap_or(14.0)
    }

    pub fn sensitive_data_policy(&self) -> &str {
        self.values
            .get("sensitive-data-policy")
            .map(|s| s.as_str())
            .unwrap_or("warn")
    }

    pub fn link_url_in_app(&self) -> bool {
        self.get_parsed("link-url-in-app").unwrap_or(true)
    }

    pub fn link_url_in_browser(&self) -> bool {
        self.get_parsed("link-url-in-browser").unwrap_or(false)
    }

    /// Get the configured plan, or None if not explicitly set by the user.
    pub fn plan(&self) -> Option<&str> {
        self.values.get("plan").map(|s| s.as_str())
    }

    /// Get the cost display scope: "session" or "all-time".
    pub fn cost_display_scope(&self) -> &str {
        self.values
            .get("cost-display-scope")
            .map(|s| s.as_str())
            .unwrap_or("all-time")
    }

    /// Get the user-configured monthly cost for their Enterprise plan.
    ///
    /// Enterprise pricing is contract-specific, so users can set their per-seat
    /// monthly cost to get accurate derived cost calculations.
    pub fn enterprise_monthly_cost(&self) -> Option<f64> {
        self.get_parsed("enterprise-monthly-cost")
    }

    /// Get the queue processing mode: "automatic", "manual", or "scheduled".
    pub fn queue_mode(&self) -> &str {
        self.values
            .get("queue-mode")
            .map(|s| s.as_str())
            .unwrap_or("automatic")
    }

    /// Get the queue schedule string (e.g. "Mon:09:00,Wed:14:00").
    pub fn queue_schedule(&self) -> &str {
        self.values
            .get("queue-schedule")
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Get the queue sandbox mode: "off", "workspace", or "strict".
    ///
    /// - `off`: no sandbox, queue tasks run with full user permissions.
    /// - `workspace`: queue tasks run inside the Seatbelt sandbox of the workspace
    ///   they were submitted from. CWD is the workspace root. Filesystem, exec, and
    ///   credential access are restricted at the kernel level.
    /// - `strict`: same as `workspace` plus network access disabled and exec restricted
    ///   to system binaries only.
    pub fn queue_sandbox_mode(&self) -> &str {
        match self.values.get("queue-sandbox").map(|s| s.as_str()) {
            Some("workspace") => "workspace",
            Some("strict") => "strict",
            // Backward compat: "true" from old configs maps to "workspace"
            Some("true") => "workspace",
            _ => "off",
        }
    }

    /// Get the working directory base for headless queue tasks.
    /// Each task gets a subdirectory `<base>/<uuid>/` under this path.
    pub fn queue_working_dir(&self) -> String {
        self.values
            .get("queue-working-dir")
            .cloned()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                    .join("thane-tasks")
                    .to_string_lossy()
                    .to_string()
            })
    }

    /// Set a config value.
    pub fn set(&mut self, key: &str, value: &str) {
        self.values.insert(key.to_string(), value.to_string());
    }

    /// Remove a config key.
    pub fn remove(&mut self, key: &str) {
        self.values.remove(key);
    }

    /// Save the config to the thane config file (`~/.config/thane/config`).
    ///
    /// Creates the directory if it doesn't exist. Writes atomically via
    /// temp file + rename.
    pub fn save(&self) -> Result<(), CoreError> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| CoreError::Generic("No config directory available".into()))?
            .join("thane");
        std::fs::create_dir_all(&config_dir)?;

        let config_path = config_dir.join("config");

        // Build output preserving existing comments/structure if the file exists,
        // or write fresh if not. For simplicity, write a clean file with all values.
        let mut lines: Vec<String> = Vec::new();
        lines.push("# thane configuration".to_string());
        lines.push("# Settings are auto-saved from the UI.".to_string());
        lines.push(String::new());

        // Sort keys for deterministic output.
        let mut keys: Vec<&String> = self.values.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(value) = self.values.get(key) {
                lines.push(format!("{key} = {value}"));
            }
        }

        // Append keybind entries.
        if !self.keybind_entries.is_empty() {
            lines.push(String::new());
            for entry in &self.keybind_entries {
                lines.push(format!("keybind = {entry}"));
            }
        }

        lines.push(String::new()); // trailing newline

        // Atomic write: temp file + rename.
        let tmp_path = config_path.with_extension("tmp");
        std::fs::write(&tmp_path, lines.join("\n"))?;
        std::fs::rename(&tmp_path, &config_path)?;

        Ok(())
    }

    /// Get all key-value pairs.
    pub fn all(&self) -> &HashMap<String, String> {
        &self.values
    }

    /// Parse user-defined keybindings from the config.
    /// Config format: `keybind = ctrl+shift+t=workspace_new`
    pub fn keybindings(&self) -> Vec<crate::keybinding::Keybinding> {
        self.keybind_entries
            .iter()
            .filter_map(|s| crate::keybinding::parse_keybind(s))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let mut config = Config::default();
        config
            .parse_content(
                r#"
# This is a comment
font-family = JetBrains Mono
font-size = 14
scrollback-limit = 5000

# Another comment
cursor-style = bar
"#,
            )
            .unwrap();

        assert_eq!(config.font_family(), "JetBrains Mono");
        assert_eq!(config.font_size(), 14.0);
        assert_eq!(config.scrollback_limit(), 5000);
        assert_eq!(config.cursor_style(), "bar");
    }

    #[test]
    fn test_defaults() {
        let config = Config::default();
        assert_eq!(config.font_family(), "JetBrains Mono NL Light");
        assert_eq!(config.font_size(), 13.0);
        assert_eq!(config.scrollback_limit(), 10000);
    }

    #[test]
    fn test_get_or() {
        let config = Config::default();
        assert_eq!(config.get_or("nonexistent", "fallback"), "fallback");
        assert_eq!(config.get_or("font-family", "fallback"), "JetBrains Mono NL Light");
    }

    #[test]
    fn test_all_returns_default_keys() {
        let config = Config::default();
        let all = config.all();
        assert!(all.contains_key("font-family"));
        assert!(all.contains_key("font-size"));
        assert!(all.contains_key("scrollback-limit"));
        assert!(all.contains_key("cursor-style"));
        assert!(all.contains_key("cursor-style-blink"));
        assert!(all.len() >= 5);
    }

    #[test]
    fn test_parse_comments_and_blank_lines() {
        let mut config = Config::default();
        config.parse_content("# full line comment\n\n  \nfont-size = 16\n# trailing comment\n").unwrap();
        assert_eq!(config.font_size(), 16.0);
    }

    #[test]
    fn test_duplicate_key_last_wins() {
        let mut config = Config::default();
        config.parse_content("font-size = 14\nfont-size = 18\n").unwrap();
        assert_eq!(config.font_size(), 18.0);
    }

    #[test]
    fn test_missing_equals_ignored() {
        let mut config = Config::default();
        // Lines without '=' should be silently ignored
        config.parse_content("no-equals-here\nfont-size = 20\n").unwrap();
        assert_eq!(config.font_size(), 20.0);
    }

    #[test]
    fn test_get_parsed_non_parseable_returns_none() {
        let mut config = Config::default();
        config.parse_content("font-size = not_a_number\n").unwrap();
        // get_parsed should return None for non-parseable values
        let parsed: Option<f64> = config.get_parsed("font-size");
        assert!(parsed.is_none());
        // font_size() accessor falls back to default 13.0
        assert_eq!(config.font_size(), 13.0);
    }

    #[test]
    fn test_set_and_get() {
        let mut config = Config::default();
        config.set("custom-key", "custom-value");
        assert_eq!(config.get("custom-key"), Some("custom-value"));
    }

    #[test]
    fn test_terminal_font_color_default() {
        let config = Config::default();
        assert_eq!(config.terminal_font_color(), "#e4e4e7");
    }

    #[test]
    fn test_terminal_font_color_custom() {
        let mut config = Config::default();
        config.set("terminal-foreground", "#ff0000");
        assert_eq!(config.terminal_font_color(), "#ff0000");
    }

    #[test]
    fn test_terminal_font_color_roundtrip() {
        let mut config = Config::default();
        config.parse_content("terminal-foreground = #aabbcc\n").unwrap();
        assert_eq!(config.terminal_font_color(), "#aabbcc");
        // Overwrite with set
        config.set("terminal-foreground", "#112233");
        assert_eq!(config.terminal_font_color(), "#112233");
    }

    #[test]
    fn test_keybind_entries_parsed() {
        let mut config = Config::default();
        config.parse_content("keybind = ctrl+shift+t=workspace_new\nkeybind = alt+h=pane_focus_left\n").unwrap();
        let bindings = config.keybindings();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].action, crate::keybinding::KeyAction::WorkspaceNew);
        assert_eq!(bindings[1].action, crate::keybinding::KeyAction::PaneFocusLeft);
    }
}
