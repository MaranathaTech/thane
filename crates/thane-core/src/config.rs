use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::CoreError;
use crate::policy::EnterprisePolicy;

/// Parsed configuration for thane.
///
/// Reads Ghostty-format config files (key = value) and provides
/// thane-specific overrides.
///
/// When an [`EnterprisePolicy`] is attached via [`Config::with_policy`],
/// keys listed in the policy's `locked_keys` ALWAYS override the user value
/// — see the precedence ladder documented on the `policy` module. The
/// user-config layer is preserved verbatim, so removing the policy restores
/// the user's choices.
#[derive(Debug, Clone)]
pub struct Config {
    /// Raw key-value pairs from the config file.
    values: HashMap<String, String>,
    /// Path the config was loaded from, if any.
    pub source_path: Option<PathBuf>,
    /// Keybinding entries (multiple values allowed for the `keybind` key).
    keybind_entries: Vec<String>,
    /// Optional enterprise policy that overrides user values for the keys it
    /// lists. `None` is the unmanaged-install case.
    policy: Option<Arc<EnterprisePolicy>>,
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
            policy: None,
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
    ///
    /// If an [`EnterprisePolicy`] is attached and locks `key`, the policy
    /// value is returned in preference to the user value.
    pub fn get(&self, key: &str) -> Option<&str> {
        if let Some(p) = self.policy.as_ref()
            && let Some(v) = p.lookup(key)
        {
            return Some(v);
        }
        self.values.get(key).map(|s| s.as_str())
    }

    /// Get a config value parsed as the given type. Honors the active
    /// [`EnterprisePolicy`].
    pub fn get_parsed<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// Get a config value or a default. Honors the active
    /// [`EnterprisePolicy`].
    pub fn get_or(&self, key: &str, default: &str) -> String {
        self.get(key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| default.to_string())
    }

    // ── Enterprise policy plumbing ──────────────────────────────────────────

    /// Attach an [`EnterprisePolicy`] to this config. Keys listed in the
    /// policy's `locked_keys` now override any user value and cannot be
    /// mutated via [`Config::set`].
    ///
    /// Internally the policy values are ALSO copied into the underlying
    /// `values` map so every existing typed accessor picks them up without
    /// each call site needing to consult the policy explicitly. The original
    /// user values are not preserved across an [`apply_policy`] call — the
    /// policy file is the source of truth while it is active. To restore the
    /// user value, remove the policy file and restart (which re-reads the
    /// user config from scratch).
    ///
    /// Builder form. Use the imperative [`Config::apply_policy`] when you
    /// already own a `&mut Config`.
    pub fn with_policy(mut self, policy: Arc<EnterprisePolicy>) -> Self {
        self.apply_policy(policy);
        self
    }

    /// Imperative variant of [`with_policy`].
    pub fn apply_policy(&mut self, policy: Arc<EnterprisePolicy>) {
        for (k, v) in &policy.locked_keys {
            self.values.insert(k.clone(), v.clone());
        }
        self.policy = Some(policy);
    }

    /// Borrow the active policy, if any.
    pub fn policy(&self) -> Option<&EnterprisePolicy> {
        self.policy.as_deref()
    }

    /// Whether `key` is locked by the active enterprise policy. UI panels
    /// use this to disable the corresponding control and show a lock icon.
    pub fn is_locked(&self, key: &str) -> bool {
        self.policy
            .as_ref()
            .map(|p| p.is_locked(key))
            .unwrap_or(false)
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

    /// Whether to audit Claude Code CLI sessions (scan JSONL project prompts).
    /// Default: true (preserves current always-on behavior).
    pub fn audit_claude_code_sessions(&self) -> bool {
        self.get_parsed("audit-claude-code-sessions").unwrap_or(true)
    }

    /// Whether to audit Claude.ai web/desktop app conversations via API.
    /// Default: false (opt-in, makes API calls using OAuth token).
    pub fn audit_claude_app_chats(&self) -> bool {
        self.get_parsed("audit-claude-app-chats").unwrap_or(false)
    }

    /// Whether to record the full prompt of each headless queue task as a UserPrompt
    /// audit event. Default: true (matches the interactive Claude Code path).
    pub fn audit_queue_prompts(&self) -> bool {
        self.get_parsed("audit-queue-prompts").unwrap_or(true)
    }

    /// Number of days to retain rotated audit log files. `0` means retain forever.
    /// Default: 90 days.
    pub fn audit_retention_days(&self) -> u32 {
        self.get_parsed("audit-retention-days").unwrap_or(90)
    }

    /// Whether the audit log Clear action is permitted at all. Default: false
    /// (enterprise compliance default — clearing requires explicit policy opt-in).
    pub fn audit_allow_clear(&self) -> bool {
        self.get_parsed("audit-allow-clear").unwrap_or(false)
    }

    /// Whether to HMAC-sign every audit event. Default: true.
    ///
    /// Setting this to `false` reverts events to the legacy unsigned + hash-chain
    /// format. Useful for tests that don't want to provision a key, or for
    /// downgrade investigation. Logged once at startup so operators can see it.
    pub fn audit_signing_enabled(&self) -> bool {
        self.get_parsed("audit-signing-enabled").unwrap_or(true)
    }

    /// Whether to AES-256-GCM encrypt rotated audit log files. Default: true.
    ///
    /// The active `audit.jsonl` is always plaintext (single-shot AEAD can't be
    /// appended). When enabled, rotated files are written as `audit.N.jsonl.enc`
    /// using a sub-key derived from the platform-stored root key. Setting this
    /// to `false` preserves the legacy plaintext rotated file layout — useful
    /// for compatibility investigation. Logged once at startup so operators
    /// can see it.
    pub fn audit_encryption_enabled(&self) -> bool {
        self.get_parsed("audit-encryption-enabled").unwrap_or(true)
    }

    /// Redaction policy applied to audit events before they hit disk.
    ///
    /// Returns the raw config value (`"none"`, `"redact"`, or `"strict"`). Unknown
    /// values map to `"redact"` so a typo never silently disables redaction.
    /// Default: `"redact"`.
    pub fn audit_redaction_policy(&self) -> crate::audit_redaction::RedactionPolicy {
        let raw = self
            .values
            .get("audit-redaction-policy")
            .map(|s| s.as_str())
            .unwrap_or("redact");
        crate::audit_redaction::RedactionPolicy::from_config_value(raw)
    }

    /// Raw string form of [`audit_redaction_policy`], for UI round-tripping.
    pub fn audit_redaction_policy_str(&self) -> &str {
        self.values
            .get("audit-redaction-policy")
            .map(|s| s.as_str())
            .unwrap_or("redact")
    }

    // ── Audit sinks (Phase 5) ───────────────────────────────────────────────
    //
    // Each external sink (syslog, webhook, ...) has an enable flag plus its
    // own per-protocol fields. Defaults are conservative: all sinks are OFF
    // until an operator explicitly turns them on.

    pub fn audit_sink_syslog_enabled(&self) -> bool {
        self.get_parsed("audit-sink-syslog-enabled").unwrap_or(false)
    }

    pub fn audit_sink_syslog_host(&self) -> Option<&str> {
        self.values.get("audit-sink-syslog-host").map(|s| s.as_str())
    }

    pub fn audit_sink_syslog_port(&self) -> u16 {
        self.get_parsed("audit-sink-syslog-port").unwrap_or(6514)
    }

    pub fn audit_sink_syslog_tls(&self) -> bool {
        self.get_parsed("audit-sink-syslog-tls").unwrap_or(true)
    }

    pub fn audit_sink_syslog_ca_cert(&self) -> Option<&str> {
        self.values
            .get("audit-sink-syslog-ca-cert")
            .map(|s| s.as_str())
    }

    pub fn audit_sink_syslog_app_name(&self) -> String {
        self.values
            .get("audit-sink-syslog-app-name")
            .cloned()
            .unwrap_or_else(|| "thane".to_string())
    }

    pub fn audit_sink_syslog_min_severity(&self) -> &str {
        self.values
            .get("audit-sink-syslog-min-severity")
            .map(|s| s.as_str())
            .unwrap_or("info")
    }

    pub fn audit_sink_webhook_enabled(&self) -> bool {
        self.get_parsed("audit-sink-webhook-enabled").unwrap_or(false)
    }

    pub fn audit_sink_webhook_url(&self) -> Option<&str> {
        self.values.get("audit-sink-webhook-url").map(|s| s.as_str())
    }

    /// Name of the secret-store entry that holds the webhook HMAC secret.
    /// Default `"thane-webhook-secret"`. The secret store must already have
    /// this entry — the sink refuses to start otherwise.
    pub fn audit_sink_webhook_secret_id(&self) -> String {
        self.values
            .get("audit-sink-webhook-secret-id")
            .cloned()
            .unwrap_or_else(|| "thane-webhook-secret".to_string())
    }

    pub fn audit_sink_webhook_min_severity(&self) -> &str {
        self.values
            .get("audit-sink-webhook-min-severity")
            .map(|s| s.as_str())
            .unwrap_or("info")
    }

    pub fn audit_sink_webhook_timeout_secs(&self) -> u64 {
        self.get_parsed("audit-sink-webhook-timeout-secs").unwrap_or(10)
    }

    // ── Audit sink: Splunk HEC ──────────────────────────────────────────────

    pub fn audit_sink_splunk_enabled(&self) -> bool {
        self.get_parsed("audit-sink-splunk-enabled").unwrap_or(false)
    }

    pub fn audit_sink_splunk_url(&self) -> Option<&str> {
        self.values.get("audit-sink-splunk-url").map(|s| s.as_str())
    }

    /// Secret-store entry holding the HEC token. Default `"thane-splunk-token"`.
    pub fn audit_sink_splunk_token_secret_id(&self) -> String {
        self.values
            .get("audit-sink-splunk-token-secret-id")
            .cloned()
            .unwrap_or_else(|| "thane-splunk-token".to_string())
    }

    pub fn audit_sink_splunk_index(&self) -> Option<&str> {
        self.values
            .get("audit-sink-splunk-index")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn audit_sink_splunk_verify_tls(&self) -> bool {
        self.get_parsed("audit-sink-splunk-verify-tls").unwrap_or(true)
    }

    pub fn audit_sink_splunk_min_severity(&self) -> &str {
        self.values
            .get("audit-sink-splunk-min-severity")
            .map(|s| s.as_str())
            .unwrap_or("info")
    }

    // ── Audit sink: Datadog Logs ────────────────────────────────────────────

    pub fn audit_sink_datadog_enabled(&self) -> bool {
        self.get_parsed("audit-sink-datadog-enabled").unwrap_or(false)
    }

    /// `us | us3 | us5 | eu | ap1`. Anything else falls back to `us`.
    pub fn audit_sink_datadog_region(&self) -> &str {
        self.values
            .get("audit-sink-datadog-region")
            .map(|s| s.as_str())
            .unwrap_or("us")
    }

    pub fn audit_sink_datadog_api_key_secret_id(&self) -> String {
        self.values
            .get("audit-sink-datadog-api-key-secret-id")
            .cloned()
            .unwrap_or_else(|| "thane-datadog-key".to_string())
    }

    pub fn audit_sink_datadog_env(&self) -> String {
        self.values
            .get("audit-sink-datadog-env")
            .cloned()
            .unwrap_or_else(|| "prod".to_string())
    }

    pub fn audit_sink_datadog_service(&self) -> String {
        self.values
            .get("audit-sink-datadog-service")
            .cloned()
            .unwrap_or_else(|| "thane".to_string())
    }

    pub fn audit_sink_datadog_min_severity(&self) -> &str {
        self.values
            .get("audit-sink-datadog-min-severity")
            .map(|s| s.as_str())
            .unwrap_or("info")
    }

    // ── Audit sink: S3 / object storage ─────────────────────────────────────

    pub fn audit_sink_s3_enabled(&self) -> bool {
        self.get_parsed("audit-sink-s3-enabled").unwrap_or(false)
    }

    pub fn audit_sink_s3_bucket(&self) -> Option<&str> {
        self.values
            .get("audit-sink-s3-bucket")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn audit_sink_s3_region(&self) -> String {
        self.values
            .get("audit-sink-s3-region")
            .cloned()
            .unwrap_or_else(|| "us-east-1".to_string())
    }

    /// Blank → AWS default. Set for Cloudflare R2, MinIO, etc.
    pub fn audit_sink_s3_endpoint(&self) -> Option<&str> {
        self.values
            .get("audit-sink-s3-endpoint")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// Secret-store ids for explicit static credentials. Both blank → SDK
    /// default credential chain (IAM role, env, ~/.aws/credentials).
    pub fn audit_sink_s3_access_key_id_secret_id(&self) -> String {
        self.values
            .get("audit-sink-s3-access-key-id-secret-id")
            .cloned()
            .unwrap_or_else(|| "thane-s3-access-key".to_string())
    }

    pub fn audit_sink_s3_secret_key_secret_id(&self) -> String {
        self.values
            .get("audit-sink-s3-secret-key-secret-id")
            .cloned()
            .unwrap_or_else(|| "thane-s3-secret-key".to_string())
    }

    pub fn audit_sink_s3_prefix(&self) -> String {
        self.values
            .get("audit-sink-s3-prefix")
            .cloned()
            .unwrap_or_else(|| "audit/".to_string())
    }

    pub fn audit_sink_s3_sse_mode(&self) -> &str {
        self.values
            .get("audit-sink-s3-sse-mode")
            .map(|s| s.as_str())
            .unwrap_or("s3")
    }

    pub fn audit_sink_s3_kms_key_id(&self) -> Option<&str> {
        self.values
            .get("audit-sink-s3-kms-key-id")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn audit_sink_s3_object_lock_mode(&self) -> &str {
        self.values
            .get("audit-sink-s3-object-lock-mode")
            .map(|s| s.as_str())
            .unwrap_or("none")
    }

    pub fn audit_sink_s3_object_lock_days(&self) -> u32 {
        self.get_parsed("audit-sink-s3-object-lock-days").unwrap_or(365)
    }

    pub fn audit_sink_s3_min_severity(&self) -> &str {
        self.values
            .get("audit-sink-s3-min-severity")
            .map(|s| s.as_str())
            .unwrap_or("info")
    }

    // ── Audit sink: Grafana Loki ────────────────────────────────────────────

    pub fn audit_sink_loki_enabled(&self) -> bool {
        self.get_parsed("audit-sink-loki-enabled").unwrap_or(false)
    }

    pub fn audit_sink_loki_url(&self) -> Option<&str> {
        self.values
            .get("audit-sink-loki-url")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// Tenant id sent as both the `tenant` label and the `X-Scope-OrgID`
    /// header. Required for multi-tenant Loki deployments (which is the only
    /// supported topology for this sink — single-tenant Loki simply ignores
    /// the header).
    pub fn audit_sink_loki_tenant(&self) -> Option<&str> {
        self.values
            .get("audit-sink-loki-tenant")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// `bearer | basic | mtls | none`. Unknown values fall back to `bearer`.
    pub fn audit_sink_loki_auth_mode(&self) -> &str {
        self.values
            .get("audit-sink-loki-auth-mode")
            .map(|s| s.as_str())
            .unwrap_or("bearer")
    }

    /// Secret-store entry that holds the Loki bearer/basic token.
    pub fn audit_sink_loki_auth_secret_id(&self) -> String {
        self.values
            .get("audit-sink-loki-auth-secret-id")
            .cloned()
            .unwrap_or_else(|| "thane-loki-token".to_string())
    }

    /// Username for Basic auth. For Grafana Cloud Logs this is typically the
    /// numeric instance id. Empty when auth mode is `bearer | mtls | none`.
    pub fn audit_sink_loki_basic_user(&self) -> Option<&str> {
        self.values
            .get("audit-sink-loki-basic-user")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn audit_sink_loki_client_cert(&self) -> Option<&str> {
        self.values
            .get("audit-sink-loki-client-cert")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn audit_sink_loki_client_key(&self) -> Option<&str> {
        self.values
            .get("audit-sink-loki-client-key")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn audit_sink_loki_ca_cert(&self) -> Option<&str> {
        self.values
            .get("audit-sink-loki-ca-cert")
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    pub fn audit_sink_loki_verify_tls(&self) -> bool {
        self.get_parsed("audit-sink-loki-verify-tls").unwrap_or(true)
    }

    pub fn audit_sink_loki_compress(&self) -> bool {
        self.get_parsed("audit-sink-loki-compress").unwrap_or(true)
    }

    pub fn audit_sink_loki_min_severity(&self) -> &str {
        self.values
            .get("audit-sink-loki-min-severity")
            .map(|s| s.as_str())
            .unwrap_or("info")
    }

    /// Set a config value.
    ///
    /// If an [`EnterprisePolicy`] locks `key`, the call is silently ignored
    /// (with a warning trace) — UI panels should consult [`Config::is_locked`]
    /// to disable the control and avoid even attempting the write.
    pub fn set(&mut self, key: &str, value: &str) {
        if self.is_locked(key) {
            tracing::warn!(
                "ignoring set of '{key}' = '{value}': locked by enterprise policy"
            );
            return;
        }
        self.values.insert(key.to_string(), value.to_string());
    }

    /// Remove a config key. No-op for keys locked by an enterprise policy.
    pub fn remove(&mut self, key: &str) {
        if self.is_locked(key) {
            tracing::warn!(
                "ignoring remove of '{key}': locked by enterprise policy"
            );
            return;
        }
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
            // Never persist a policy-locked key into the user config — if the
            // policy is later removed we don't want a stale lock value to
            // shadow the user's real preference.
            if self.is_locked(key) {
                continue;
            }
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

    #[test]
    fn test_audit_claude_code_sessions_default() {
        let config = Config::default();
        assert!(config.audit_claude_code_sessions());
    }

    #[test]
    fn test_audit_claude_app_chats_default() {
        let config = Config::default();
        assert!(!config.audit_claude_app_chats());
    }

    #[test]
    fn test_audit_claude_code_sessions_explicit_false() {
        let mut config = Config::default();
        config.set("audit-claude-code-sessions", "false");
        assert!(!config.audit_claude_code_sessions());
    }

    #[test]
    fn test_audit_claude_app_chats_explicit_true() {
        let mut config = Config::default();
        config.set("audit-claude-app-chats", "true");
        assert!(config.audit_claude_app_chats());
    }

    #[test]
    fn test_audit_queue_prompts_default_true() {
        let config = Config::default();
        assert!(config.audit_queue_prompts());
    }

    #[test]
    fn test_audit_queue_prompts_opt_out() {
        let mut config = Config::default();
        config.set("audit-queue-prompts", "false");
        assert!(!config.audit_queue_prompts());
    }

    #[test]
    fn test_audit_retention_days_default_90() {
        let config = Config::default();
        assert_eq!(config.audit_retention_days(), 90);
    }

    #[test]
    fn test_audit_retention_days_zero_means_forever() {
        let mut config = Config::default();
        config.set("audit-retention-days", "0");
        assert_eq!(config.audit_retention_days(), 0);
    }

    #[test]
    fn test_audit_retention_days_custom() {
        let mut config = Config::default();
        config.set("audit-retention-days", "30");
        assert_eq!(config.audit_retention_days(), 30);
    }

    #[test]
    fn test_audit_allow_clear_default_false() {
        let config = Config::default();
        assert!(!config.audit_allow_clear());
    }

    #[test]
    fn test_audit_allow_clear_explicit_true() {
        let mut config = Config::default();
        config.set("audit-allow-clear", "true");
        assert!(config.audit_allow_clear());
    }

    #[test]
    fn test_audit_signing_enabled_default_true() {
        let config = Config::default();
        assert!(config.audit_signing_enabled());
    }

    #[test]
    fn test_audit_signing_enabled_explicit_false() {
        let mut config = Config::default();
        config.set("audit-signing-enabled", "false");
        assert!(!config.audit_signing_enabled());
    }

    #[test]
    fn test_audit_encryption_enabled_default_true() {
        let config = Config::default();
        assert!(config.audit_encryption_enabled());
    }

    #[test]
    fn test_audit_encryption_enabled_explicit_false() {
        let mut config = Config::default();
        config.set("audit-encryption-enabled", "false");
        assert!(!config.audit_encryption_enabled());
    }

    #[test]
    fn test_audit_redaction_policy_default_redact() {
        let config = Config::default();
        assert_eq!(
            config.audit_redaction_policy(),
            crate::audit_redaction::RedactionPolicy::Redact
        );
        assert_eq!(config.audit_redaction_policy_str(), "redact");
    }

    #[test]
    fn test_audit_redaction_policy_explicit_strict() {
        let mut config = Config::default();
        config.set("audit-redaction-policy", "strict");
        assert_eq!(
            config.audit_redaction_policy(),
            crate::audit_redaction::RedactionPolicy::Strict
        );
        assert_eq!(config.audit_redaction_policy_str(), "strict");
    }

    #[test]
    fn test_audit_redaction_policy_explicit_none() {
        let mut config = Config::default();
        config.set("audit-redaction-policy", "none");
        assert_eq!(
            config.audit_redaction_policy(),
            crate::audit_redaction::RedactionPolicy::None
        );
    }

    #[test]
    fn test_audit_redaction_policy_unknown_falls_back_to_redact() {
        // Typo guard — must NOT silently disable redaction.
        let mut config = Config::default();
        config.set("audit-redaction-policy", "verbose");
        assert_eq!(
            config.audit_redaction_policy(),
            crate::audit_redaction::RedactionPolicy::Redact
        );
    }

    // ── Enterprise policy override tests ────────────────────────────────────

    fn make_policy(pairs: &[(&str, &str)]) -> std::sync::Arc<crate::policy::EnterprisePolicy> {
        let locked_keys = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        std::sync::Arc::new(crate::policy::EnterprisePolicy {
            policy_version: 1,
            issued_by: "Acme Corp".into(),
            issued_at: "2026-05-25".into(),
            locked_keys,
            ui_banner: Some("Enterprise audit policy active".into()),
        })
    }

    #[test]
    fn policy_overrides_user_value_for_locked_keys() {
        let mut config = Config::default();
        config.set("audit-sink-loki-enabled", "false"); // user value
        let policy = make_policy(&[("audit-sink-loki-enabled", "true")]);
        config.apply_policy(policy);
        // Policy wins via both the generic get and the typed accessor.
        assert_eq!(config.get("audit-sink-loki-enabled"), Some("true"));
        assert!(config.audit_sink_loki_enabled());
        assert!(config.is_locked("audit-sink-loki-enabled"));
    }

    #[test]
    fn non_locked_keys_still_read_from_user_config() {
        let mut config = Config::default();
        config.set("audit-retention-days", "42"); // user value
        let policy = make_policy(&[("audit-sink-loki-enabled", "true")]);
        config.apply_policy(policy);
        assert_eq!(config.audit_retention_days(), 42);
        assert!(!config.is_locked("audit-retention-days"));
    }

    #[test]
    fn set_on_locked_key_is_silently_ignored() {
        let mut config = Config::default();
        let policy = make_policy(&[("audit-allow-clear", "false")]);
        config.apply_policy(policy);
        // User tries to flip the lock — must be silently no-op.
        config.set("audit-allow-clear", "true");
        assert_eq!(config.get("audit-allow-clear"), Some("false"));
        assert!(!config.audit_allow_clear());
    }

    #[test]
    fn remove_on_locked_key_is_silently_ignored() {
        let mut config = Config::default();
        let policy = make_policy(&[("audit-allow-clear", "false")]);
        config.apply_policy(policy);
        config.remove("audit-allow-clear");
        assert_eq!(config.get("audit-allow-clear"), Some("false"));
    }

    #[test]
    fn save_skips_policy_locked_keys() {
        // Indirect: serialize the values that *would* be saved by iterating
        // the same logic.
        let mut config = Config::default();
        config.set("audit-retention-days", "30");
        let policy = make_policy(&[("audit-sink-loki-enabled", "true")]);
        config.apply_policy(policy);
        // Manually mimic save's loop.
        let mut written: Vec<String> = Vec::new();
        let mut keys: Vec<&String> = config.values.keys().collect();
        keys.sort();
        for key in keys {
            if config.is_locked(key) {
                continue;
            }
            if let Some(v) = config.values.get(key) {
                written.push(format!("{key} = {v}"));
            }
        }
        let joined = written.join("\n");
        assert!(joined.contains("audit-retention-days = 30"));
        assert!(!joined.contains("audit-sink-loki-enabled"));
    }

    #[test]
    fn is_locked_returns_false_when_no_policy() {
        let config = Config::default();
        assert!(!config.is_locked("audit-sink-loki-enabled"));
        assert!(!config.is_locked("anything"));
        assert!(config.policy().is_none());
    }
}
