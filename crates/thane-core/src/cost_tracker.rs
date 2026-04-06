use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Token usage and cost tracking for Claude Code sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostTracker {
    /// Total input tokens used in the current session.
    pub input_tokens: u64,
    /// Total output tokens used in the current session.
    pub output_tokens: u64,
    /// Total cache read tokens.
    pub cache_read_tokens: u64,
    /// Total cache write tokens.
    pub cache_write_tokens: u64,
    /// Estimated total cost in USD.
    pub estimated_cost_usd: f64,
}

/// Detailed cost summary for a project, separating current session from all-time.
#[derive(Debug, Clone, Default)]
pub struct ProjectCostSummary {
    /// Entries with timestamp >= thane's `started_at`.
    pub current_session: CostTracker,
    /// All entries, no filter.
    pub all_time: CostTracker,
    /// Per-JSONL-file breakdown.
    pub sessions: Vec<SessionCost>,
}

/// Cost data for a single Claude Code session (one JSONL file).
#[derive(Debug, Clone)]
pub struct SessionCost {
    /// Filename stem (session identifier).
    pub session_id: String,
    /// Earliest timestamp found in this file.
    pub first_timestamp: Option<DateTime<Utc>>,
    /// Latest timestamp found in this file.
    pub last_timestamp: Option<DateTime<Utc>>,
    /// Aggregated cost for this file.
    pub cost: CostTracker,
}

/// Cached cost tracker that avoids re-parsing unchanged JSONL files.
///
/// Stores per-file parsed entries keyed by `(path, mtime_ns, size)`.
/// On each call to `for_project_detailed`, only files whose mtime or size
/// changed since the last scan are re-read and re-parsed.
#[derive(Debug, Default)]
pub struct CostCache {
    /// Cached per-file data: path → (mtime_ns, size, parsed_entries).
    files: HashMap<PathBuf, CachedFile>,
}

#[derive(Debug, Clone)]
struct CachedFile {
    mtime_ns: u128,
    size: u64,
    entries: Vec<CachedUsageEntry>,
}

/// A cached usage entry with all fields needed to rebuild the summary.
#[derive(Debug, Clone)]
struct CachedUsageEntry {
    timestamp: Option<DateTime<Utc>>,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
}

impl From<&ParsedUsageEntry> for CachedUsageEntry {
    fn from(pe: &ParsedUsageEntry) -> Self {
        Self {
            timestamp: pe.timestamp,
            model: pe.model.clone(),
            input_tokens: pe.input_tokens,
            output_tokens: pe.output_tokens,
            cache_read_tokens: pe.cache_read_tokens,
            cache_write_tokens: pe.cache_write_tokens,
        }
    }
}

impl CachedUsageEntry {
    fn as_parsed(&self) -> ParsedUsageEntry {
        ParsedUsageEntry {
            timestamp: self.timestamp,
            model: self.model.clone(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens,
        }
    }
}

impl CostCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scan for Claude Code usage with incremental caching.
    ///
    /// Scans the CWD **and all ancestor CWDs** that have matching Claude project
    /// directories. Only re-parses JSONL files that have changed since the last call.
    pub fn for_project_detailed(&mut self, cwd: &str, since: Option<DateTime<Utc>>) -> ProjectCostSummary {
        let mut summary = ProjectCostSummary::default();

        // Collect current files across all matching project directories.
        let mut current_files: HashMap<PathBuf, (u128, u64)> = HashMap::new();
        for project_dir in project_dirs_for_cwd(cwd) {
            let dir_entries = match std::fs::read_dir(&project_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in dir_entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "jsonl") {
                    continue;
                }
                if let Ok(meta) = path.metadata() {
                    let mtime_ns = meta.modified().ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    current_files.insert(path, (mtime_ns, meta.len()));
                }
            }
        }

        // Remove stale cache entries for files that no longer exist.
        self.files.retain(|path, _| current_files.contains_key(path));

        // Update cache for changed files, then build the summary.
        for (path, (mtime_ns, size)) in &current_files {
            let needs_reparse = match self.files.get(path) {
                Some(cached) => cached.mtime_ns != *mtime_ns || cached.size != *size,
                None => true,
            };

            if needs_reparse {
                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let parsed = parse_jsonl_content_detailed(&content);
                let cached_entries: Vec<CachedUsageEntry> = parsed.iter().map(CachedUsageEntry::from).collect();
                self.files.insert(path.clone(), CachedFile {
                    mtime_ns: *mtime_ns,
                    size: *size,
                    entries: cached_entries,
                });
            }

            let cached = &self.files[path];
            if cached.entries.is_empty() {
                continue;
            }

            let session_id = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            let mut session_cost = CostTracker::new();
            let mut first_ts: Option<DateTime<Utc>> = None;
            let mut last_ts: Option<DateTime<Utc>> = None;

            for ce in &cached.entries {
                let pe = ce.as_parsed();
                summary.all_time.add_parsed_entry(&pe);
                session_cost.add_parsed_entry(&pe);

                if let Some(t) = ce.timestamp {
                    first_ts = Some(first_ts.map_or(t, |prev: DateTime<Utc>| prev.min(t)));
                    last_ts = Some(last_ts.map_or(t, |prev: DateTime<Utc>| prev.max(t)));

                    if since.is_none() || since.is_some_and(|s| t >= s) {
                        summary.current_session.add_parsed_entry(&pe);
                    }
                } else {
                    let include = if let Some(since_dt) = since {
                        let file_mtime_dt = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(*mtime_ns as u64);
                        file_mtime_dt.duration_since(std::time::UNIX_EPOCH).ok()
                            .and_then(|d| DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos()))
                            .is_some_and(|mt| mt >= since_dt)
                    } else {
                        true
                    };
                    if include {
                        summary.current_session.add_parsed_entry(&pe);
                    }
                }
            }

            summary.sessions.push(SessionCost {
                session_id,
                first_timestamp: first_ts,
                last_timestamp: last_ts,
                cost: session_cost,
            });
        }

        summary
    }
}

/// Whether to display utilization percentage or dollar cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostDisplayMode {
    /// Show utilization % as primary metric (subscription plans with caps).
    Utilization,
    /// Show dollar cost as primary metric (API/Enterprise or no OAuth data).
    Dollar,
}

/// Anthropic plan type for token limit tracking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    #[default]
    Pro,
    Max5,
    Max20,
    Team,
    Enterprise,
    Api,
}

impl Plan {
    /// Parse a plan string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Self {
        let lower = s.to_lowercase();
        match lower.as_str() {
            "pro" => Plan::Pro,
            "max5" | "max_5" | "max (5x)" => Plan::Max5,
            "max20" | "max_20" | "max (20x)" => Plan::Max20,
            "team" => Plan::Team,
            "enterprise" => Plan::Enterprise,
            "api" => Plan::Api,
            _ if lower.starts_with("max") => {
                // Handle "max" alone — disambiguate using rateLimitTier if available.
                if let Some(tier) = read_rate_limit_tier() {
                    if tier.contains("20x") {
                        Plan::Max20
                    } else {
                        Plan::Max5
                    }
                } else {
                    Plan::Max5
                }
            }
            _ => Plan::Pro,
        }
    }

    /// Display name for this plan.
    pub fn display_name(&self) -> &'static str {
        match self {
            Plan::Pro => "Pro",
            Plan::Max5 => "Max (5x)",
            Plan::Max20 => "Max (20x)",
            Plan::Team => "Team",
            Plan::Enterprise => "Enterprise",
            Plan::Api => "API",
        }
    }

    /// Whether this plan has usage caps.
    pub fn has_caps(&self) -> bool {
        matches!(self, Plan::Pro | Plan::Max5 | Plan::Max20 | Plan::Team)
    }

    /// Monthly subscription price in USD, if applicable.
    ///
    /// Returns `None` for API (pay-per-token). For Enterprise, returns `None`
    /// unless an override is provided (since Enterprise pricing is contract-specific).
    pub fn monthly_price_usd(&self) -> Option<f64> {
        match self {
            Plan::Pro => Some(20.0),
            Plan::Max5 => Some(100.0),
            Plan::Max20 => Some(200.0),
            Plan::Team => Some(30.0),
            Plan::Enterprise | Plan::Api => None,
        }
    }

    /// Default Enterprise monthly cost estimate when user hasn't configured one.
    ///
    /// Defaults to $200/seat/month (worst-case, matching Max 20x pricing).
    /// Users can override this in Settings with their actual contract rate.
    pub const ENTERPRISE_DEFAULT_MONTHLY_USD: f64 = 200.0;

    /// Monthly price with an optional Enterprise override from user config.
    ///
    /// For Enterprise plans, uses the override if provided, otherwise falls back
    /// to the default estimate.
    pub fn monthly_price_usd_with_override(&self, enterprise_override: Option<f64>) -> Option<f64> {
        match self {
            Plan::Enterprise => Some(
                enterprise_override.unwrap_or(Self::ENTERPRISE_DEFAULT_MONTHLY_USD),
            ),
            _ => self.monthly_price_usd(),
        }
    }

    /// Auto-detect the plan from recent Claude Code session data.
    ///
    /// Heuristic: scans recent JSONL files for model usage patterns.
    /// - Predominantly Opus → Max (defaults to Max5)
    /// - Predominantly Sonnet → Pro
    /// - No data → defaults to Pro
    ///
    /// If `config_override` is set (i.e. user explicitly configured a plan), that
    /// takes precedence.
    pub fn detect(config_override: Option<&str>) -> Self {
        // If the user explicitly set a plan in config, respect it.
        if let Some(s) = config_override
            && !s.is_empty()
        {
            return Self::from_str_loose(s);
        }

        Self::detect_from_sessions()
    }

    /// Scan recent Claude Code sessions to infer the plan type.
    fn detect_from_sessions() -> Self {
        let claude_dir = default_claude_dir();
        let projects_dir = claude_dir.join("projects");
        if !projects_dir.is_dir() {
            return Plan::Pro;
        }

        let mut opus_count: u64 = 0;
        let mut sonnet_count: u64 = 0;

        // Scan the most recent JSONL files (by mtime) across all projects.
        let mut jsonl_files: Vec<PathBuf> = Vec::new();
        if let Ok(project_dirs) = std::fs::read_dir(&projects_dir) {
            for dir_entry in project_dirs.flatten() {
                let dir = dir_entry.path();
                if !dir.is_dir() {
                    continue;
                }
                if let Ok(files) = std::fs::read_dir(&dir) {
                    for file_entry in files.flatten() {
                        let path = file_entry.path();
                        if path.extension().is_some_and(|ext| ext == "jsonl") {
                            jsonl_files.push(path);
                        }
                    }
                }
            }
        }

        // Sort by mtime descending, take most recent 5 files.
        jsonl_files.sort_by(|a, b| {
            let ma = a.metadata().ok().and_then(|m| m.modified().ok());
            let mb = b.metadata().ok().and_then(|m| m.modified().ok());
            mb.cmp(&ma)
        });

        for path in jsonl_files.iter().take(5) {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                if let Ok(entry) = serde_json::from_str::<JournalEntry>(line)
                    && entry.r#type == "assistant"
                    && let Some(msg) = &entry.message
                {
                    if msg.model.contains("opus") {
                        opus_count += 1;
                    } else if msg.model.contains("sonnet") {
                        sonnet_count += 1;
                    }
                }
            }
        }

        if opus_count == 0 && sonnet_count == 0 {
            return Plan::Pro;
        }

        // Predominantly Opus → Max plan (we can't distinguish 5x vs 20x from
        // model usage alone, default to Max5).
        if opus_count > sonnet_count * 2 {
            Plan::Max5
        } else {
            Plan::Pro
        }
    }
}

/// Token limit information from the Anthropic OAuth usage API.
#[derive(Debug, Clone, Default)]
pub struct TokenLimitInfo {
    /// Current plan.
    pub plan: Plan,
    /// 5-hour rolling window utilization (0–100%).
    pub five_hour: Option<UsageWindow>,
    /// 7-day rolling window utilization (0–100%).
    pub seven_day: Option<UsageWindow>,
    /// Whether the plan has usage caps.
    pub has_caps: bool,
}

/// A usage window from the Anthropic OAuth usage API.
#[derive(Debug, Clone)]
pub struct UsageWindow {
    /// Utilization percentage (0.0–100.0).
    pub utilization: f64,
    /// When this window resets.
    pub resets_at: DateTime<Utc>,
}

/// Response from the Anthropic OAuth usage API.
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthUsageResponse {
    pub five_hour: Option<OAuthUsageWindow>,
    pub seven_day: Option<OAuthUsageWindow>,
}

/// A single window from the OAuth usage response.
#[derive(Debug, Clone, Deserialize)]
pub struct OAuthUsageWindow {
    pub utilization: f64,
    pub resets_at: String,
}

/// Read the OAuth access token from ~/.claude/.credentials.json.
pub fn read_oauth_token() -> Option<String> {
    let creds_path = default_claude_dir().join(".credentials.json");
    let content = std::fs::read_to_string(&creds_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()
        .map(|s| s.to_string())
}

/// Read the rate limit tier from ~/.claude/.credentials.json.
pub fn read_rate_limit_tier() -> Option<String> {
    let creds_path = default_claude_dir().join(".credentials.json");
    let content = std::fs::read_to_string(&creds_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("claudeAiOauth")?
        .get("rateLimitTier")?
        .as_str()
        .map(|s| s.to_string())
}

/// Read the subscription type from ~/.claude/.credentials.json.
pub fn read_subscription_type() -> Option<String> {
    let creds_path = default_claude_dir().join(".credentials.json");
    let content = std::fs::read_to_string(&creds_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("claudeAiOauth")?
        .get("subscriptionType")?
        .as_str()
        .map(|s| s.to_string())
}

/// Fetch usage data from the Anthropic OAuth API using curl.
///
/// Returns `None` on network errors, auth failures, or rate limiting (429).
/// This is a blocking call — run from a background thread.
pub fn fetch_oauth_usage(access_token: &str) -> Option<OAuthUsageResponse> {
    let output = std::process::Command::new("curl")
        .args([
            "-s", "-f",
            "-H", &format!("Authorization: Bearer {access_token}"),
            "-H", "anthropic-beta: oauth-2025-04-20",
            "https://api.anthropic.com/api/oauth/usage",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    serde_json::from_slice(&output.stdout).ok()
}

impl TokenLimitInfo {
    /// Construct limit info from an OAuth usage response.
    pub fn from_oauth(plan: Plan, response: &OAuthUsageResponse) -> Self {
        let five_hour = response.five_hour.as_ref().and_then(|w| {
            parse_timestamp(&w.resets_at).map(|resets_at| UsageWindow {
                utilization: w.utilization,
                resets_at,
            })
        });
        let seven_day = response.seven_day.as_ref().and_then(|w| {
            parse_timestamp(&w.resets_at).map(|resets_at| UsageWindow {
                utilization: w.utilization,
                resets_at,
            })
        });

        Self {
            plan,
            five_hour,
            seven_day,
            has_caps: plan.has_caps(),
        }
    }

    /// Determine whether to show utilization or dollar cost as the primary metric.
    ///
    /// Returns `Utilization` for capped plans (Pro/Max/Team) when usage data is available.
    /// Returns `Dollar` for API/Enterprise plans or when no OAuth usage data exists.
    pub fn display_mode(&self) -> CostDisplayMode {
        if self.has_caps && (self.five_hour.is_some() || self.seven_day.is_some()) {
            CostDisplayMode::Utilization
        } else {
            CostDisplayMode::Dollar
        }
    }

    /// Like `display_mode` but includes Enterprise plans (which have a default cost).
    ///
    /// Enterprise plans always show utilization when usage data is available,
    /// because they have a default monthly cost estimate ($200) that users can
    /// override in Settings.
    pub fn display_mode_with_override(
        &self,
        _enterprise_monthly_cost: Option<f64>,
    ) -> CostDisplayMode {
        let has_usage = self.five_hour.is_some() || self.seven_day.is_some();
        if !has_usage {
            return CostDisplayMode::Dollar;
        }
        if self.has_caps {
            return CostDisplayMode::Utilization;
        }
        // Enterprise always shows utilization (has default cost estimate).
        if self.plan == Plan::Enterprise {
            return CostDisplayMode::Utilization;
        }
        CostDisplayMode::Dollar
    }

    /// The most relevant utilization percentage: 5-hour if available, else 7-day.
    pub fn primary_utilization(&self) -> Option<f64> {
        self.five_hour
            .as_ref()
            .map(|w| w.utilization)
            .or_else(|| self.seven_day.as_ref().map(|w| w.utilization))
    }

    /// Derive an effective cost from the subscription price and utilization.
    ///
    /// For subscription plans: `monthly_price × (utilization / 100)`.
    /// Returns `None` for API (no monthly price) or when no utilization data.
    /// For Enterprise, uses the user-configured monthly cost if provided.
    pub fn derived_subscription_cost(&self) -> Option<f64> {
        let price = self.plan.monthly_price_usd()?;
        let utilization = self.primary_utilization()?;
        Some(price * utilization / 100.0)
    }

    /// Like `derived_subscription_cost` but accepts an Enterprise override.
    pub fn derived_subscription_cost_with_override(
        &self,
        enterprise_monthly_cost: Option<f64>,
    ) -> Option<f64> {
        let price = self
            .plan
            .monthly_price_usd_with_override(enterprise_monthly_cost)?;
        let utilization = self.primary_utilization()?;
        Some(price * utilization / 100.0)
    }
}

/// Per-model pricing (input/output per million tokens, USD).
#[derive(Debug, Clone)]
struct ModelPricing {
    input_per_million: f64,
    output_per_million: f64,
    cache_read_per_million: f64,
    cache_write_per_million: f64,
}

/// Default pricing for Claude Opus 4.
static OPUS_PRICING: ModelPricing = ModelPricing {
    input_per_million: 15.0,
    output_per_million: 75.0,
    cache_read_per_million: 1.5,
    cache_write_per_million: 18.75,
};

/// Default pricing for Claude Sonnet 4.
static SONNET_PRICING: ModelPricing = ModelPricing {
    input_per_million: 3.0,
    output_per_million: 15.0,
    cache_read_per_million: 0.3,
    cache_write_per_million: 3.75,
};

fn pricing_for_model(model: &str) -> &'static ModelPricing {
    if model.contains("opus") {
        &OPUS_PRICING
    } else {
        // Default to Sonnet pricing (most commonly used).
        &SONNET_PRICING
    }
}

/// A parsed usage entry with optional timestamp.
#[derive(Debug)]
struct ParsedUsageEntry {
    timestamp: Option<DateTime<Utc>>,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
}

/// A journal entry from Claude Code's JSONL output.
/// Claude Code nests usage data inside `message.usage` for "assistant" entries.
#[derive(Debug, Clone, Deserialize)]
struct JournalEntry {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    message: Option<JournalMessage>,
    /// ISO 8601 timestamp (some Claude Code versions include this at the top level).
    #[serde(default)]
    timestamp: Option<String>,
}

/// The `message` field inside a journal entry.
#[derive(Debug, Clone, Deserialize)]
struct JournalMessage {
    #[serde(default)]
    model: String,
    #[serde(default)]
    usage: Option<JournalUsage>,
}

/// Token usage counts from Claude Code's journal format (snake_case).
#[derive(Debug, Clone, Deserialize)]
struct JournalUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

/// Legacy flat usage record format (camelCase) for forward-compatibility.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageRecord {
    #[serde(default)]
    model: String,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    timestamp: Option<String>,
}

/// Parse an ISO 8601 timestamp string into DateTime<Utc>.
fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    s.parse::<DateTime<Utc>>().ok()
}

/// Parse JSONL content into individual usage entries with timestamps.
fn parse_jsonl_content_detailed(content: &str) -> Vec<ParsedUsageEntry> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try the nested journal format (Claude Code's actual output).
        if let Ok(entry) = serde_json::from_str::<JournalEntry>(line)
            && entry.r#type == "assistant"
            && let Some(msg) = &entry.message
            && let Some(usage) = &msg.usage
        {
            let timestamp = entry.timestamp.as_deref().and_then(parse_timestamp);
            entries.push(ParsedUsageEntry {
                timestamp,
                model: msg.model.clone(),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
            });
            continue;
        }

        // Fall back to flat camelCase format.
        if let Ok(record) = serde_json::from_str::<UsageRecord>(line)
            && (record.input_tokens > 0 || record.output_tokens > 0)
        {
            let timestamp = record.timestamp.as_deref().and_then(parse_timestamp);
            entries.push(ParsedUsageEntry {
                timestamp,
                model: record.model,
                input_tokens: record.input_tokens,
                output_tokens: record.output_tokens,
                cache_read_tokens: record.cache_read_input_tokens,
                cache_write_tokens: record.cache_creation_input_tokens,
            });
        }
    }

    entries
}

impl CostTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse Claude Code usage from a projects directory.
    /// Looks for JSONL files in ~/.claude/projects/<project>/ that contain usage data.
    pub fn from_claude_dir(claude_dir: &Path) -> Self {
        let mut tracker = Self::new();

        // Try to read from the projects directory (Claude Code stores per-project usage).
        let projects_dir = claude_dir.join("projects");
        if projects_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&projects_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    tracker.scan_project_dir(&path, None);
                }
            }
        }

        tracker
    }

    /// Scan a specific project directory for JSONL usage files.
    fn scan_project_dir(&mut self, project_dir: &Path, since: Option<std::time::SystemTime>) {
        if let Ok(entries) = std::fs::read_dir(project_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "jsonl") {
                    // If a `since` filter is set, skip files not modified since then.
                    if let Some(since) = since {
                        let dominated = path.metadata().ok()
                            .and_then(|m| m.modified().ok())
                            .is_some_and(|mtime| mtime < since);
                        if dominated {
                            continue;
                        }
                    }
                    self.parse_jsonl_file(&path);
                }
            }
        }
    }

    /// Parse a JSONL file containing usage records.
    fn parse_jsonl_file(&mut self, path: &Path) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        self.parse_jsonl_content(&content);
    }

    /// Parse JSONL content string containing usage records.
    ///
    /// Tries Claude Code's nested journal format first (`type: "assistant"` with
    /// `message.usage`), then falls back to a flat `UsageRecord` for compatibility.
    fn parse_jsonl_content(&mut self, content: &str) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Try the nested journal format (Claude Code's actual output).
            if let Ok(entry) = serde_json::from_str::<JournalEntry>(line)
                && entry.r#type == "assistant"
                && let Some(msg) = &entry.message
                && let Some(usage) = &msg.usage
            {
                self.add_usage(
                    &msg.model,
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_input_tokens,
                    usage.cache_creation_input_tokens,
                );
                continue;
            }

            // Fall back to flat camelCase format.
            if let Ok(record) = serde_json::from_str::<UsageRecord>(line)
                && (record.input_tokens > 0 || record.output_tokens > 0)
            {
                self.add_usage(
                    &record.model,
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_input_tokens,
                    record.cache_creation_input_tokens,
                );
            }
        }
    }

    /// Add token usage and cost for a single record.
    fn add_usage(&mut self, model: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) {
        let pricing = pricing_for_model(model);
        self.input_tokens += input;
        self.output_tokens += output;
        self.cache_read_tokens += cache_read;
        self.cache_write_tokens += cache_write;
        self.estimated_cost_usd +=
            (input as f64 * pricing.input_per_million / 1_000_000.0)
            + (output as f64 * pricing.output_per_million / 1_000_000.0)
            + (cache_read as f64 * pricing.cache_read_per_million / 1_000_000.0)
            + (cache_write as f64 * pricing.cache_write_per_million / 1_000_000.0);
    }

    /// Add entries from a `ParsedUsageEntry`.
    fn add_parsed_entry(&mut self, entry: &ParsedUsageEntry) {
        self.add_usage(
            &entry.model,
            entry.input_tokens,
            entry.output_tokens,
            entry.cache_read_tokens,
            entry.cache_write_tokens,
        );
    }

    /// Scan for Claude Code usage for a specific project CWD.
    /// Returns a CostTracker with aggregated usage for that project.
    /// If `since` is provided, only JSONL files modified after that time are scanned.
    pub fn for_project(cwd: &str, since: Option<std::time::SystemTime>) -> Self {
        let since_dt = since.and_then(|s| {
            s.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|d| DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos()))
        });
        let summary = Self::for_project_detailed(cwd, since_dt);
        summary.current_session
    }

    /// Scan for Claude Code usage for a specific project CWD with detailed breakdown.
    ///
    /// Scans the CWD **and all ancestor CWDs** that have matching Claude project
    /// directories, so that sessions started from a project root are still found
    /// when the terminal has `cd`'d into a subdirectory.
    ///
    /// Returns a `ProjectCostSummary` with:
    /// - `current_session`: only entries with timestamp >= `since`
    /// - `all_time`: all entries regardless of timestamp
    /// - `sessions`: per-JSONL-file breakdown
    pub fn for_project_detailed(cwd: &str, since: Option<DateTime<Utc>>) -> ProjectCostSummary {
        let mut summary = ProjectCostSummary::default();
        let mut seen_files = std::collections::HashSet::new();

        for project_dir in project_dirs_for_cwd(cwd) {
            let entries = match std::fs::read_dir(&project_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "jsonl") {
                    continue;
                }
                if !seen_files.insert(path.clone()) {
                    continue;
                }

                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let parsed = parse_jsonl_content_detailed(&content);
                if parsed.is_empty() {
                    continue;
                }

                let session_id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();

                let mut session_cost = CostTracker::new();
                let mut first_ts: Option<DateTime<Utc>> = None;
                let mut last_ts: Option<DateTime<Utc>> = None;

                for pe in &parsed {
                    summary.all_time.add_parsed_entry(pe);
                    session_cost.add_parsed_entry(pe);

                    if let Some(t) = pe.timestamp {
                        first_ts = Some(first_ts.map_or(t, |prev: DateTime<Utc>| prev.min(t)));
                        last_ts = Some(last_ts.map_or(t, |prev: DateTime<Utc>| prev.max(t)));

                        if since.is_none() || since.is_some_and(|s| t >= s) {
                            summary.current_session.add_parsed_entry(pe);
                        }
                    } else {
                        let include = if let Some(since) = since {
                            path.metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|mtime| {
                                    mtime.duration_since(std::time::UNIX_EPOCH).ok().and_then(|d| {
                                        DateTime::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
                                    })
                                })
                                .is_some_and(|mtime_dt| mtime_dt >= since)
                        } else {
                            true
                        };
                        if include {
                            summary.current_session.add_parsed_entry(pe);
                        }
                    }
                }

                summary.sessions.push(SessionCost {
                    session_id,
                    first_timestamp: first_ts,
                    last_timestamp: last_ts,
                    cost: session_cost,
                });
            }
        }

        summary
    }

    /// Format cost as a human-readable string.
    pub fn format_cost(&self) -> String {
        if self.estimated_cost_usd < 0.01 {
            "<$0.01".to_string()
        } else {
            format!("${:.2}", self.estimated_cost_usd)
        }
    }

    /// Format total tokens as a human-readable string.
    pub fn format_tokens(&self) -> String {
        let total = self.input_tokens + self.output_tokens;
        if total < 1_000 {
            format!("{total} tokens")
        } else if total < 1_000_000 {
            format!("{:.1}K tokens", total as f64 / 1_000.0)
        } else {
            format!("{:.1}M tokens", total as f64 / 1_000_000.0)
        }
    }
}

/// Return all `~/.claude/projects/<mangled>/` directories that correspond to
/// the given CWD **or any of its ancestor directories**.
///
/// This ensures that when a terminal has `cd`'d into a subdirectory (e.g.
/// `/a/b/c/marketing`), we still pick up sessions started from `/a/b/c`.
fn project_dirs_for_cwd(cwd: &str) -> Vec<PathBuf> {
    let projects_dir = default_claude_dir().join("projects");
    if !projects_dir.is_dir() {
        return Vec::new();
    }

    let mut dirs = Vec::new();
    let mut path = Path::new(cwd);

    for _ in 0..20 {
        let mangled = path.to_string_lossy().replace('/', "-");
        let project_dir = projects_dir.join(&mangled);
        if project_dir.is_dir() {
            dirs.push(project_dir);
        }
        match path.parent() {
            Some(parent) if parent != path => path = parent,
            _ => break,
        }
    }

    dirs
}

/// Get the default Claude Code directory (~/.claude).
fn default_claude_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".claude")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_tracker() {
        let tracker = CostTracker::new();
        assert_eq!(tracker.input_tokens, 0);
        assert_eq!(tracker.output_tokens, 0);
        assert_eq!(tracker.estimated_cost_usd, 0.0);
        assert_eq!(tracker.format_cost(), "<$0.01");
        assert_eq!(tracker.format_tokens(), "0 tokens");
    }

    #[test]
    fn test_format_cost() {
        let mut tracker = CostTracker::new();
        tracker.estimated_cost_usd = 0.005;
        assert_eq!(tracker.format_cost(), "<$0.01");

        tracker.estimated_cost_usd = 0.15;
        assert_eq!(tracker.format_cost(), "$0.15");

        tracker.estimated_cost_usd = 2.50;
        assert_eq!(tracker.format_cost(), "$2.50");
    }

    #[test]
    fn test_format_tokens() {
        let mut tracker = CostTracker::new();
        tracker.input_tokens = 500;
        tracker.output_tokens = 300;
        assert_eq!(tracker.format_tokens(), "800 tokens");

        tracker.input_tokens = 50_000;
        tracker.output_tokens = 10_000;
        assert_eq!(tracker.format_tokens(), "60.0K tokens");

        tracker.input_tokens = 1_500_000;
        tracker.output_tokens = 500_000;
        assert_eq!(tracker.format_tokens(), "2.0M tokens");
    }

    #[test]
    fn test_pricing_selection() {
        assert!(std::ptr::eq(
            pricing_for_model("claude-opus-4"),
            &OPUS_PRICING
        ));
        assert!(std::ptr::eq(
            pricing_for_model("claude-sonnet-4"),
            &SONNET_PRICING
        ));
        assert!(std::ptr::eq(
            pricing_for_model("unknown-model"),
            &SONNET_PRICING
        ));
    }

    #[test]
    fn test_for_nonexistent_project() {
        let tracker = CostTracker::for_project("/nonexistent/path/that/does/not/exist", None);
        assert_eq!(tracker.input_tokens, 0);
        assert_eq!(tracker.estimated_cost_usd, 0.0);
    }

    #[test]
    fn test_parse_single_usage_record() {
        let mut tracker = CostTracker::new();
        let content = r#"{"model":"claude-sonnet-4","inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 1000);
        assert_eq!(tracker.output_tokens, 500);
        // Sonnet pricing: 1000 * 3.0 / 1M + 500 * 15.0 / 1M = 0.003 + 0.0075 = 0.0105
        assert!((tracker.estimated_cost_usd - 0.0105).abs() < 1e-10);
    }

    #[test]
    fn test_parse_multiple_records_accumulate() {
        let mut tracker = CostTracker::new();
        let content = r#"{"model":"claude-sonnet-4","inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}
{"model":"claude-sonnet-4","inputTokens":2000,"outputTokens":1000,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 3000);
        assert_eq!(tracker.output_tokens, 1500);
    }

    #[test]
    fn test_parse_mixed_models() {
        let mut tracker = CostTracker::new();
        let content = r#"{"model":"claude-opus-4","inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}
{"model":"claude-sonnet-4","inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 2000);
        assert_eq!(tracker.output_tokens, 1000);
        // Opus: 1000*15/1M + 500*75/1M = 0.015 + 0.0375 = 0.0525
        // Sonnet: 1000*3/1M + 500*15/1M = 0.003 + 0.0075 = 0.0105
        // Total: 0.063
        assert!((tracker.estimated_cost_usd - 0.063).abs() < 1e-10);
    }

    #[test]
    fn test_parse_malformed_lines_skipped() {
        let mut tracker = CostTracker::new();
        let content = r#"not json at all
{"some":"other","json":"object"}
{"model":"claude-sonnet-4","inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":0,"cacheCreationInputTokens":0}
{broken json"#;
        tracker.parse_jsonl_content(content);
        // Only the valid usage record should count. The "other json object" will parse
        // as a UsageRecord with all defaults (0 tokens, empty model), contributing 0 cost.
        assert_eq!(tracker.input_tokens, 1000);
        assert_eq!(tracker.output_tokens, 500);
    }

    #[test]
    fn test_parse_empty_and_whitespace_lines() {
        let mut tracker = CostTracker::new();
        let content = "\n   \n\n";
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 0);
        assert_eq!(tracker.estimated_cost_usd, 0.0);
    }

    #[test]
    fn test_parse_with_cache_tokens() {
        let mut tracker = CostTracker::new();
        let content = r#"{"model":"claude-sonnet-4","inputTokens":1000,"outputTokens":500,"cacheReadInputTokens":2000,"cacheCreationInputTokens":3000}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.cache_read_tokens, 2000);
        assert_eq!(tracker.cache_write_tokens, 3000);
        // Sonnet: 1000*3/1M + 500*15/1M + 2000*0.3/1M + 3000*3.75/1M
        //       = 0.003 + 0.0075 + 0.0006 + 0.01125 = 0.02235
        assert!((tracker.estimated_cost_usd - 0.02235).abs() < 1e-10);
    }

    #[test]
    fn test_parse_nested_journal_format() {
        let mut tracker = CostTracker::new();
        let content = r#"{"type":"assistant","message":{"model":"claude-opus-4-6","role":"assistant","usage":{"input_tokens":3,"output_tokens":27,"cache_read_input_tokens":16561,"cache_creation_input_tokens":8391}}}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 3);
        assert_eq!(tracker.output_tokens, 27);
        assert_eq!(tracker.cache_read_tokens, 16561);
        assert_eq!(tracker.cache_write_tokens, 8391);
        // Opus pricing: 3*15/1M + 27*75/1M + 16561*1.5/1M + 8391*18.75/1M
        let expected = (3.0 * 15.0 + 27.0 * 75.0 + 16561.0 * 1.5 + 8391.0 * 18.75) / 1_000_000.0;
        assert!((tracker.estimated_cost_usd - expected).abs() < 1e-10);
    }

    #[test]
    fn test_parse_nested_multiple_assistants() {
        let mut tracker = CostTracker::new();
        let content = r#"{"type":"user","message":{"role":"user","content":"hello"}}
{"type":"assistant","message":{"model":"claude-opus-4-6","role":"assistant","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}
{"type":"user","message":{"role":"user","content":"more"}}
{"type":"assistant","message":{"model":"claude-opus-4-6","role":"assistant","usage":{"input_tokens":200,"output_tokens":100,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 300);
        assert_eq!(tracker.output_tokens, 150);
    }

    #[test]
    fn test_parse_nested_non_assistant_ignored() {
        let mut tracker = CostTracker::new();
        // "user" type entries should not contribute tokens even if they have usage fields
        let content = r#"{"type":"user","message":{"role":"user","content":"hello"}}
{"type":"file-history-snapshot","messageId":"abc"}
{"type":"assistant","message":{"model":"claude-sonnet-4","role":"assistant","usage":{"input_tokens":500,"output_tokens":200,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 500);
        assert_eq!(tracker.output_tokens, 200);
    }

    #[test]
    fn test_parse_nested_mixed_models() {
        let mut tracker = CostTracker::new();
        let content = r#"{"type":"assistant","message":{"model":"claude-opus-4-6","role":"assistant","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}
{"type":"assistant","message":{"model":"claude-sonnet-4-5-20250929","role":"assistant","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        tracker.parse_jsonl_content(content);
        assert_eq!(tracker.input_tokens, 2000);
        assert_eq!(tracker.output_tokens, 1000);
        // Opus: 1000*15/1M + 500*75/1M = 0.015 + 0.0375 = 0.0525
        // Sonnet: 1000*3/1M + 500*15/1M = 0.003 + 0.0075 = 0.0105
        // Total: 0.063
        assert!((tracker.estimated_cost_usd - 0.063).abs() < 1e-10);
    }

    // ── New tests for detailed cost tracking ──

    #[test]
    fn test_for_project_detailed_timestamp_filtering() {
        // Test that entries before `since` only appear in all_time, not current_session.
        let now = Utc::now();
        let old_ts = (now - chrono::Duration::hours(2)).to_rfc3339();
        let new_ts = (now - chrono::Duration::minutes(5)).to_rfc3339();
        let since = now - chrono::Duration::hours(1);

        let content = format!(
            r#"{{"type":"assistant","timestamp":"{old_ts}","message":{{"model":"claude-sonnet-4","role":"assistant","usage":{{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}
{{"type":"assistant","timestamp":"{new_ts}","message":{{"model":"claude-sonnet-4","role":"assistant","usage":{{"input_tokens":2000,"output_tokens":1000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        );

        let entries = parse_jsonl_content_detailed(&content);
        assert_eq!(entries.len(), 2);

        // Build summary manually (simulating what for_project_detailed does).
        let mut summary = ProjectCostSummary::default();
        for pe in &entries {
            summary.all_time.add_parsed_entry(pe);
            if let Some(t) = pe.timestamp {
                if t >= since {
                    summary.current_session.add_parsed_entry(pe);
                }
            }
        }

        // All-time should have both entries.
        assert_eq!(summary.all_time.input_tokens, 3000);
        assert_eq!(summary.all_time.output_tokens, 1500);

        // Current session should only have the new entry.
        assert_eq!(summary.current_session.input_tokens, 2000);
        assert_eq!(summary.current_session.output_tokens, 1000);
    }

    #[test]
    fn test_session_cost_per_file() {
        // Each JSONL file should produce its own SessionCost.
        let content1 = r#"{"type":"assistant","message":{"model":"claude-sonnet-4","role":"assistant","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        let content2 = r#"{"type":"assistant","message":{"model":"claude-sonnet-4","role":"assistant","usage":{"input_tokens":200,"output_tokens":100,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;

        let entries1 = parse_jsonl_content_detailed(content1);
        let entries2 = parse_jsonl_content_detailed(content2);

        let mut cost1 = CostTracker::new();
        for e in &entries1 {
            cost1.add_parsed_entry(e);
        }

        let mut cost2 = CostTracker::new();
        for e in &entries2 {
            cost2.add_parsed_entry(e);
        }

        assert_eq!(cost1.input_tokens, 100);
        assert_eq!(cost1.output_tokens, 50);
        assert_eq!(cost2.input_tokens, 200);
        assert_eq!(cost2.output_tokens, 100);
    }

    #[test]
    fn test_token_limit_info_from_oauth() {
        // OAuthUsageResponse and OAuthUsageWindow are imported via `use super::*`.

        let response = OAuthUsageResponse {
            five_hour: Some(OAuthUsageWindow {
                utilization: 25.0,
                resets_at: "2025-03-21T15:00:00Z".to_string(),
            }),
            seven_day: Some(OAuthUsageWindow {
                utilization: 42.0,
                resets_at: "2025-03-27T10:00:00Z".to_string(),
            }),
        };

        let info = TokenLimitInfo::from_oauth(Plan::Pro, &response);
        assert!(info.has_caps);
        assert!(info.five_hour.is_some());
        assert!(info.seven_day.is_some());
        let fh = info.five_hour.unwrap();
        assert!((fh.utilization - 25.0).abs() < 0.001);
        let sd = info.seven_day.unwrap();
        assert!((sd.utilization - 42.0).abs() < 0.001);
    }

    #[test]
    fn test_token_limit_info_from_oauth_no_caps_plan() {
        // OAuthUsageResponse is imported via `use super::*`.

        let response = OAuthUsageResponse {
            five_hour: None,
            seven_day: None,
        };

        let info = TokenLimitInfo::from_oauth(Plan::Enterprise, &response);
        assert!(!info.has_caps);
        assert!(info.five_hour.is_none());
        assert!(info.seven_day.is_none());
    }

    #[test]
    fn test_token_limit_info_from_oauth_bad_timestamp() {
        // OAuthUsageResponse and OAuthUsageWindow are imported via `use super::*`.

        let response = OAuthUsageResponse {
            five_hour: Some(OAuthUsageWindow {
                utilization: 10.0,
                resets_at: "not-a-valid-timestamp".to_string(),
            }),
            seven_day: None,
        };

        let info = TokenLimitInfo::from_oauth(Plan::Pro, &response);
        // Bad timestamp → five_hour is None.
        assert!(info.five_hour.is_none());
    }

    #[test]
    fn test_plan_from_str_loose() {
        assert_eq!(Plan::from_str_loose("pro"), Plan::Pro);
        assert_eq!(Plan::from_str_loose("Pro"), Plan::Pro);
        assert_eq!(Plan::from_str_loose("max5"), Plan::Max5);
        assert_eq!(Plan::from_str_loose("Max (5x)"), Plan::Max5);
        assert_eq!(Plan::from_str_loose("max20"), Plan::Max20);
        assert_eq!(Plan::from_str_loose("enterprise"), Plan::Enterprise);
        assert_eq!(Plan::from_str_loose("api"), Plan::Api);
        assert_eq!(Plan::from_str_loose("team"), Plan::Team);
        assert_eq!(Plan::from_str_loose("unknown"), Plan::Pro); // default
    }

    #[test]
    fn test_plan_display_name() {
        assert_eq!(Plan::Pro.display_name(), "Pro");
        assert_eq!(Plan::Max5.display_name(), "Max (5x)");
        assert_eq!(Plan::Enterprise.display_name(), "Enterprise");
    }

    #[test]
    fn test_plan_has_caps() {
        assert!(Plan::Pro.has_caps());
        assert!(Plan::Max5.has_caps());
        assert!(Plan::Max20.has_caps());
        assert!(Plan::Team.has_caps());
        assert!(!Plan::Enterprise.has_caps());
        assert!(!Plan::Api.has_caps());
    }

    #[test]
    fn test_parse_jsonl_content_detailed_with_timestamps() {
        let ts = "2025-03-20T10:00:00Z";
        let content = format!(
            r#"{{"type":"assistant","timestamp":"{ts}","message":{{"model":"claude-sonnet-4","role":"assistant","usage":{{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
        );

        let entries = parse_jsonl_content_detailed(&content);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].timestamp.is_some());
        assert_eq!(entries[0].input_tokens, 100);
    }

    #[test]
    fn test_parse_jsonl_content_detailed_without_timestamps() {
        let content = r#"{"type":"assistant","message":{"model":"claude-sonnet-4","role":"assistant","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;

        let entries = parse_jsonl_content_detailed(content);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].timestamp.is_none());
        assert_eq!(entries[0].input_tokens, 100);
    }

    // ── Display mode tests ──

    #[test]
    fn test_display_mode_utilization_when_capped_with_data() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: Some(UsageWindow {
                utilization: 42.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: true,
        };
        assert_eq!(info.display_mode(), CostDisplayMode::Utilization);
    }

    #[test]
    fn test_display_mode_dollar_when_capped_without_data() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: None,
            seven_day: None,
            has_caps: true,
        };
        assert_eq!(info.display_mode(), CostDisplayMode::Dollar);
    }

    #[test]
    fn test_display_mode_dollar_for_api_plan() {
        let info = TokenLimitInfo {
            plan: Plan::Api,
            five_hour: Some(UsageWindow {
                utilization: 10.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: false,
        };
        assert_eq!(info.display_mode(), CostDisplayMode::Dollar);
    }

    #[test]
    fn test_display_mode_dollar_for_enterprise() {
        let info = TokenLimitInfo {
            plan: Plan::Enterprise,
            five_hour: None,
            seven_day: None,
            has_caps: false,
        };
        assert_eq!(info.display_mode(), CostDisplayMode::Dollar);
    }

    #[test]
    fn test_display_mode_utilization_seven_day_only() {
        let info = TokenLimitInfo {
            plan: Plan::Max5,
            five_hour: None,
            seven_day: Some(UsageWindow {
                utilization: 60.0,
                resets_at: Utc::now(),
            }),
            has_caps: true,
        };
        assert_eq!(info.display_mode(), CostDisplayMode::Utilization);
    }

    #[test]
    fn test_primary_utilization_prefers_five_hour() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: Some(UsageWindow {
                utilization: 30.0,
                resets_at: Utc::now(),
            }),
            seven_day: Some(UsageWindow {
                utilization: 50.0,
                resets_at: Utc::now(),
            }),
            has_caps: true,
        };
        assert!((info.primary_utilization().unwrap() - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_primary_utilization_falls_back_to_seven_day() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: None,
            seven_day: Some(UsageWindow {
                utilization: 50.0,
                resets_at: Utc::now(),
            }),
            has_caps: true,
        };
        assert!((info.primary_utilization().unwrap() - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_primary_utilization_none_when_no_data() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: None,
            seven_day: None,
            has_caps: true,
        };
        assert!(info.primary_utilization().is_none());
    }

    // ── Monthly price tests ──

    #[test]
    fn test_monthly_price_pro() {
        assert!((Plan::Pro.monthly_price_usd().unwrap() - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_monthly_price_max5() {
        assert!((Plan::Max5.monthly_price_usd().unwrap() - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_monthly_price_max20() {
        assert!((Plan::Max20.monthly_price_usd().unwrap() - 200.0).abs() < 0.001);
    }

    #[test]
    fn test_monthly_price_team() {
        assert!((Plan::Team.monthly_price_usd().unwrap() - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_monthly_price_enterprise_none() {
        assert!(Plan::Enterprise.monthly_price_usd().is_none());
    }

    #[test]
    fn test_monthly_price_api_none() {
        assert!(Plan::Api.monthly_price_usd().is_none());
    }

    // ── Derived subscription cost tests ──

    #[test]
    fn test_derived_cost_pro_at_50_percent() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: Some(UsageWindow {
                utilization: 50.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: true,
        };
        // Pro $20 × 50% = $10.00
        let cost = info.derived_subscription_cost().unwrap();
        assert!((cost - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_derived_cost_max5_at_25_percent() {
        let info = TokenLimitInfo {
            plan: Plan::Max5,
            five_hour: Some(UsageWindow {
                utilization: 25.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: true,
        };
        // Max5 $100 × 25% = $25.00
        let cost = info.derived_subscription_cost().unwrap();
        assert!((cost - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_derived_cost_max20_at_100_percent() {
        let info = TokenLimitInfo {
            plan: Plan::Max20,
            five_hour: Some(UsageWindow {
                utilization: 100.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: true,
        };
        // Max20 $200 × 100% = $200.00
        let cost = info.derived_subscription_cost().unwrap();
        assert!((cost - 200.0).abs() < 0.001);
    }

    #[test]
    fn test_derived_cost_uses_seven_day_when_no_five_hour() {
        let info = TokenLimitInfo {
            plan: Plan::Team,
            five_hour: None,
            seven_day: Some(UsageWindow {
                utilization: 80.0,
                resets_at: Utc::now(),
            }),
            has_caps: true,
        };
        // Team $30 × 80% = $24.00
        let cost = info.derived_subscription_cost().unwrap();
        assert!((cost - 24.0).abs() < 0.001);
    }

    #[test]
    fn test_derived_cost_none_for_enterprise() {
        let info = TokenLimitInfo {
            plan: Plan::Enterprise,
            five_hour: Some(UsageWindow {
                utilization: 50.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: false,
        };
        // Enterprise has no monthly price → None
        assert!(info.derived_subscription_cost().is_none());
    }

    #[test]
    fn test_derived_cost_none_for_api() {
        let info = TokenLimitInfo {
            plan: Plan::Api,
            five_hour: None,
            seven_day: None,
            has_caps: false,
        };
        assert!(info.derived_subscription_cost().is_none());
    }

    #[test]
    fn test_derived_cost_none_when_no_utilization() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: None,
            seven_day: None,
            has_caps: true,
        };
        // Has monthly price but no utilization data → None
        assert!(info.derived_subscription_cost().is_none());
    }

    #[test]
    fn test_derived_cost_zero_at_zero_percent() {
        let info = TokenLimitInfo {
            plan: Plan::Pro,
            five_hour: Some(UsageWindow {
                utilization: 0.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: true,
        };
        // Pro $20 × 0% = $0.00
        let cost = info.derived_subscription_cost().unwrap();
        assert!((cost - 0.0).abs() < 0.001);
    }

    // ── Enterprise override tests ──

    #[test]
    fn test_enterprise_default_monthly_price() {
        // Enterprise uses default $200 when no override
        let price = Plan::Enterprise.monthly_price_usd_with_override(None);
        assert!((price.unwrap() - 200.0).abs() < 0.001);
    }

    #[test]
    fn test_enterprise_custom_monthly_price() {
        let price = Plan::Enterprise.monthly_price_usd_with_override(Some(500.0));
        assert!((price.unwrap() - 500.0).abs() < 0.001);
    }

    #[test]
    fn test_pro_ignores_enterprise_override() {
        // Non-Enterprise plans should ignore the override
        let price = Plan::Pro.monthly_price_usd_with_override(Some(500.0));
        assert!((price.unwrap() - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_enterprise_derived_cost_with_default() {
        let info = TokenLimitInfo {
            plan: Plan::Enterprise,
            five_hour: Some(UsageWindow {
                utilization: 50.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: false,
        };
        // Enterprise $200 default × 50% = $100.00
        let cost = info.derived_subscription_cost_with_override(None).unwrap();
        assert!((cost - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_enterprise_derived_cost_with_custom_override() {
        let info = TokenLimitInfo {
            plan: Plan::Enterprise,
            five_hour: Some(UsageWindow {
                utilization: 25.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: false,
        };
        // Enterprise $500 custom × 25% = $125.00
        let cost = info
            .derived_subscription_cost_with_override(Some(500.0))
            .unwrap();
        assert!((cost - 125.0).abs() < 0.001);
    }

    #[test]
    fn test_enterprise_display_mode_with_usage_data() {
        let info = TokenLimitInfo {
            plan: Plan::Enterprise,
            five_hour: Some(UsageWindow {
                utilization: 30.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: false,
        };
        // Enterprise always shows utilization when data is available
        assert_eq!(
            info.display_mode_with_override(None),
            CostDisplayMode::Utilization
        );
    }

    #[test]
    fn test_enterprise_display_mode_without_usage_data() {
        let info = TokenLimitInfo {
            plan: Plan::Enterprise,
            five_hour: None,
            seven_day: None,
            has_caps: false,
        };
        // No usage data → dollar mode
        assert_eq!(
            info.display_mode_with_override(None),
            CostDisplayMode::Dollar
        );
    }

    #[test]
    fn test_api_display_mode_still_dollar() {
        let info = TokenLimitInfo {
            plan: Plan::Api,
            five_hour: Some(UsageWindow {
                utilization: 50.0,
                resets_at: Utc::now(),
            }),
            seven_day: None,
            has_caps: false,
        };
        // API plan should always be dollar
        assert_eq!(
            info.display_mode_with_override(None),
            CostDisplayMode::Dollar
        );
    }

}
