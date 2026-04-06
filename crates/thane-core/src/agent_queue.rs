use std::collections::VecDeque;

use chrono::{DateTime, Datelike, Local, Timelike, Utc, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sandbox::SandboxPolicy;

/// Queue processing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueProcessingMode {
    /// Automatically process queued tasks as soon as possible.
    Automatic,
    /// Only process when the user explicitly triggers it.
    Manual,
    /// Process during scheduled time windows.
    Scheduled,
}

impl QueueProcessingMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "manual" => Self::Manual,
            "scheduled" => Self::Scheduled,
            _ => Self::Automatic,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
        }
    }
}

/// A single schedule entry (day + time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleEntry {
    pub day: Weekday,
    pub hour: u32,
    pub minute: u32,
}

/// Parse a schedule string like "Mon:09:00,Wed:14:00,Fri:18:00".
pub fn parse_schedule(s: &str) -> Vec<ScheduleEntry> {
    let s = s.trim();
    if s.is_empty() {
        return Vec::new();
    }
    s.split(',')
        .filter_map(|part| {
            let part = part.trim();
            let (day_str, time_str) = part.split_once(':')?;
            let day = match day_str.trim().to_lowercase().as_str() {
                "mon" => Weekday::Mon,
                "tue" => Weekday::Tue,
                "wed" => Weekday::Wed,
                "thu" => Weekday::Thu,
                "fri" => Weekday::Fri,
                "sat" => Weekday::Sat,
                "sun" => Weekday::Sun,
                _ => return None,
            };
            let (h_str, m_str) = time_str.split_once(':')?;
            let hour: u32 = h_str.trim().parse().ok()?;
            let minute: u32 = m_str.trim().parse().ok()?;
            if hour < 24 && minute < 60 {
                Some(ScheduleEntry { day, hour, minute })
            } else {
                None
            }
        })
        .collect()
}

/// Check if the current local time is within 5 minutes of any schedule entry.
pub fn is_within_schedule(schedule: &[ScheduleEntry]) -> bool {
    let now = Local::now();
    let now_weekday = now.weekday();
    let now_minutes = now.hour() * 60 + now.minute();

    for entry in schedule {
        if entry.day == now_weekday {
            let entry_minutes = entry.hour * 60 + entry.minute;
            // Within a 5-minute window after the scheduled time.
            if now_minutes >= entry_minutes && now_minutes < entry_minutes + 5 {
                return true;
            }
        }
    }
    false
}

/// Status of an entry in the agent execution queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueEntryStatus {
    /// Waiting to be executed.
    Queued,
    /// Currently running.
    Running,
    /// Paused due to token limit exhaustion.
    PausedTokenLimit,
    /// Paused by user request.
    PausedByUser,
    /// Successfully completed.
    Completed,
    /// Failed with an error.
    Failed,
    /// Cancelled by user.
    Cancelled,
}

/// A task submitted for execution by a Claude Code agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntry {
    pub id: Uuid,
    /// The task content — could be markdown, structured JSON, or a file path.
    pub content: String,
    /// Which workspace to execute in (creates a new one if None).
    pub workspace_id: Option<Uuid>,
    /// Priority (higher values run first).
    pub priority: i32,
    pub status: QueueEntryStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message if the task failed.
    pub error: Option<String>,
    /// Token usage tracking for this task.
    pub tokens_used: QueueTokenUsage,
    /// Optional dependency: this entry will not run until the dependency completes successfully.
    #[serde(default)]
    pub depends_on: Option<Uuid>,
}

/// Token usage for a queue entry execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_usd: f64,
    /// The model Claude Code selected for this task (e.g. "claude-sonnet-4-5-20250514").
    #[serde(default)]
    pub model: Option<String>,
}

/// The agent execution queue.
///
/// Tasks are executed in priority order (highest first), then FIFO
/// within the same priority. The queue pauses execution when token
/// limits are hit and resumes automatically when the limit resets.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentQueue {
    entries: VecDeque<QueueEntry>,
    /// Whether execution is paused due to token limit.
    pub token_limit_paused: bool,
    /// When the token limit is expected to reset.
    pub token_limit_resets_at: Option<DateTime<Utc>>,
    /// Runtime flag: set by "Process All" button, reset when queue drains.
    /// Not persisted — user intent shouldn't survive restarts.
    #[serde(skip)]
    pub process_all: bool,
    /// Queue-level sandbox policy for headless task execution.
    /// When enabled, all queue tasks run within this sandbox regardless of workspace.
    #[serde(default)]
    pub sandbox_policy: SandboxPolicy,
}

impl AgentQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a reference to the queue-level sandbox policy.
    pub fn sandbox_policy(&self) -> &SandboxPolicy {
        &self.sandbox_policy
    }

    /// Get a mutable reference to the queue-level sandbox policy.
    pub fn sandbox_policy_mut(&mut self) -> &mut SandboxPolicy {
        &mut self.sandbox_policy
    }

    /// Submit a new task to the queue.
    pub fn submit(
        &mut self,
        content: String,
        workspace_id: Option<Uuid>,
        priority: i32,
    ) -> Uuid {
        self.submit_with_depends(content, workspace_id, priority, None)
    }

    /// Submit a new task with an optional dependency on another entry.
    ///
    /// If `depends_on` is `Some(id)`, this entry will not be considered runnable
    /// until the dependency has completed successfully. If the dependency fails
    /// or is cancelled, this entry will also be failed.
    pub fn submit_with_depends(
        &mut self,
        content: String,
        workspace_id: Option<Uuid>,
        priority: i32,
        depends_on: Option<Uuid>,
    ) -> Uuid {
        let entry = QueueEntry {
            id: Uuid::new_v4(),
            content,
            workspace_id,
            priority,
            status: QueueEntryStatus::Queued,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            error: None,
            tokens_used: QueueTokenUsage::default(),
            depends_on,
        };
        let id = entry.id;
        self.entries.push_back(entry);
        // Sort by priority (stable, so FIFO within same priority).
        self.entries
            .make_contiguous()
            .sort_by(|a, b| b.priority.cmp(&a.priority));
        id
    }

    /// Get the next entry ready for execution.
    ///
    /// Skips entries whose dependency has not yet completed successfully.
    pub fn next_runnable(&self) -> Option<&QueueEntry> {
        if self.token_limit_paused {
            return None;
        }
        self.entries.iter().find(|p| {
            p.status == QueueEntryStatus::Queued && self.dependency_satisfied(p)
        })
    }

    /// Check whether an entry's dependency (if any) has been satisfied.
    fn dependency_satisfied(&self, entry: &QueueEntry) -> bool {
        match entry.depends_on {
            None => true,
            Some(dep_id) => {
                self.entries
                    .iter()
                    .find(|e| e.id == dep_id)
                    .is_some_and(|dep| dep.status == QueueEntryStatus::Completed)
            }
        }
    }

    /// Mark an entry as running.
    pub fn start(&mut self, entry_id: Uuid) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|p| p.id == entry_id) {
            entry.status = QueueEntryStatus::Running;
            entry.started_at = Some(Utc::now());
            true
        } else {
            false
        }
    }

    /// Mark an entry as completed.
    pub fn complete(&mut self, entry_id: Uuid) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|p| p.id == entry_id) {
            entry.status = QueueEntryStatus::Completed;
            entry.completed_at = Some(Utc::now());
            true
        } else {
            false
        }
    }

    /// Mark an entry as failed, and cascade failure to any entries that depend on it.
    pub fn fail(&mut self, entry_id: Uuid, error: String) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|p| p.id == entry_id) {
            entry.status = QueueEntryStatus::Failed;
            entry.completed_at = Some(Utc::now());
            entry.error = Some(error);
        } else {
            return false;
        }
        self.cascade_failure(entry_id);
        true
    }

    /// Fail all queued entries that transitively depend on `failed_id`.
    fn cascade_failure(&mut self, failed_id: Uuid) {
        let now = Utc::now();
        // Collect IDs to cascade (may be transitive chains).
        let mut to_fail: Vec<Uuid> = Vec::new();
        let mut frontier = vec![failed_id];
        while let Some(id) = frontier.pop() {
            for entry in &self.entries {
                if entry.depends_on == Some(id)
                    && entry.status == QueueEntryStatus::Queued
                    && !to_fail.contains(&entry.id)
                {
                    to_fail.push(entry.id);
                    frontier.push(entry.id);
                }
            }
        }
        for id in to_fail {
            if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
                entry.status = QueueEntryStatus::Failed;
                entry.completed_at = Some(now);
                entry.error = Some(format!("Dependency {} failed", failed_id));
            }
        }
    }

    /// Remove an entry from the queue entirely.
    pub fn remove(&mut self, entry_id: Uuid) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.id != entry_id);
        self.entries.len() < len_before
    }

    /// Retry a failed or cancelled entry by resetting it to Queued.
    pub fn retry(&mut self, entry_id: Uuid) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == entry_id)
            && (entry.status == QueueEntryStatus::Failed || entry.status == QueueEntryStatus::Cancelled)
        {
            entry.status = QueueEntryStatus::Queued;
            entry.started_at = None;
            entry.completed_at = None;
            entry.error = None;
            return true;
        }
        false
    }

    /// Cancel an entry, and cascade failure to any entries that depend on it.
    pub fn cancel(&mut self, entry_id: Uuid) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|p| p.id == entry_id) {
            entry.status = QueueEntryStatus::Cancelled;
            entry.completed_at = Some(Utc::now());
        } else {
            return false;
        }
        self.cascade_failure(entry_id);
        true
    }

    /// Pause execution due to token limit exhaustion.
    pub fn pause_for_token_limit(&mut self, resets_at: DateTime<Utc>) {
        self.token_limit_paused = true;
        self.token_limit_resets_at = Some(resets_at);

        // Pause any currently running entry.
        for entry in &mut self.entries {
            if entry.status == QueueEntryStatus::Running {
                entry.status = QueueEntryStatus::PausedTokenLimit;
            }
        }
    }

    /// Resume execution after token limit reset.
    pub fn resume_after_token_reset(&mut self) {
        self.token_limit_paused = false;
        self.token_limit_resets_at = None;

        // Re-queue any token-paused entries.
        for entry in &mut self.entries {
            if entry.status == QueueEntryStatus::PausedTokenLimit {
                entry.status = QueueEntryStatus::Queued;
            }
        }
    }

    /// Check if the token limit has reset and resume if so.
    pub fn check_token_limit_reset(&mut self) -> bool {
        if let Some(resets_at) = self.token_limit_resets_at
            && Utc::now() >= resets_at
        {
            self.resume_after_token_reset();
            return true;
        }
        false
    }

    /// Update token usage for a running entry.
    pub fn update_tokens(&mut self, entry_id: Uuid, usage: QueueTokenUsage) {
        if let Some(entry) = self.entries.iter_mut().find(|p| p.id == entry_id) {
            entry.tokens_used = usage;
        }
    }

    /// Get an entry by ID.
    pub fn get(&self, entry_id: Uuid) -> Option<&QueueEntry> {
        self.entries.iter().find(|p| p.id == entry_id)
    }

    /// List all entries.
    pub fn list(&self) -> &VecDeque<QueueEntry> {
        &self.entries
    }

    /// Get queued entry count.
    pub fn queued_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|p| p.status == QueueEntryStatus::Queued)
            .count()
    }

    /// Get running entry count.
    pub fn running_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|p| p.status == QueueEntryStatus::Running)
            .count()
    }

    /// Whether the queue should automatically process the next entry,
    /// given the current processing mode and schedule.
    pub fn should_auto_process(&self, mode: QueueProcessingMode, schedule: &[ScheduleEntry]) -> bool {
        match mode {
            QueueProcessingMode::Automatic => true,
            QueueProcessingMode::Manual => self.process_all,
            QueueProcessingMode::Scheduled => self.process_all || is_within_schedule(schedule),
        }
    }

    /// Return entries with a terminal status (Completed, Failed, Cancelled).
    pub fn completed_entries(&self) -> Vec<&QueueEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(
                e.status,
                QueueEntryStatus::Completed | QueueEntryStatus::Failed | QueueEntryStatus::Cancelled
            ))
            .collect()
    }

    /// Remove finished entries whose `completed_at` is older than `max_age`.
    ///
    /// Returns the number of entries removed.
    pub fn remove_stale(&mut self, max_age: chrono::Duration) -> usize {
        let cutoff = Utc::now() - max_age;
        let len_before = self.entries.len();
        self.entries.retain(|e| {
            !matches!(
                e.status,
                QueueEntryStatus::Completed | QueueEntryStatus::Failed | QueueEntryStatus::Cancelled
            ) || e.completed_at.is_none_or(|t| t > cutoff)
        });
        len_before - self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_and_list() {
        let mut queue = AgentQueue::new();
        let id1 = queue.submit("Task A".into(), None, 0);
        let id2 = queue.submit("Task B".into(), None, 10);

        assert_eq!(queue.list().len(), 2);
        // Task B should be first (higher priority).
        assert_eq!(queue.list()[0].id, id2);
        assert_eq!(queue.list()[1].id, id1);
    }

    #[test]
    fn test_execution_lifecycle() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);

        assert_eq!(queue.queued_count(), 1);

        let next = queue.next_runnable().unwrap();
        assert_eq!(next.id, id);

        queue.start(id);
        assert_eq!(queue.running_count(), 1);
        assert_eq!(queue.queued_count(), 0);

        queue.complete(id);
        assert_eq!(queue.running_count(), 0);
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Completed);
    }

    #[test]
    fn test_token_limit_pause_resume() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.start(id);

        // Pause for token limit.
        let reset_time = Utc::now() + chrono::Duration::hours(1);
        queue.pause_for_token_limit(reset_time);

        assert!(queue.token_limit_paused);
        assert_eq!(
            queue.get(id).unwrap().status,
            QueueEntryStatus::PausedTokenLimit
        );
        assert!(queue.next_runnable().is_none());

        // Resume.
        queue.resume_after_token_reset();
        assert!(!queue.token_limit_paused);
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Queued);
        assert!(queue.next_runnable().is_some());
    }

    #[test]
    fn test_cancel() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);

        queue.cancel(id);
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Cancelled);
        assert!(queue.next_runnable().is_none());
    }

    #[test]
    fn test_fail_sets_error() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.start(id);

        queue.fail(id, "token limit exceeded".into());
        let entry = queue.get(id).unwrap();
        assert_eq!(entry.status, QueueEntryStatus::Failed);
        assert_eq!(entry.error.as_deref(), Some("token limit exceeded"));
        assert!(entry.completed_at.is_some());
    }

    #[test]
    fn test_update_tokens() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.start(id);

        let usage = QueueTokenUsage {
            input_tokens: 5000,
            output_tokens: 2000,
            estimated_cost_usd: 0.15,
            model: Some("claude-sonnet-4-5-20250514".to_string()),
        };
        queue.update_tokens(id, usage);

        let entry = queue.get(id).unwrap();
        assert_eq!(entry.tokens_used.input_tokens, 5000);
        assert_eq!(entry.tokens_used.output_tokens, 2000);
        assert!((entry.tokens_used.estimated_cost_usd - 0.15).abs() < f64::EPSILON);
    }

    #[test]
    fn test_next_runnable_none_when_paused() {
        let mut queue = AgentQueue::new();
        queue.submit("Task A".into(), None, 0);

        let reset_time = Utc::now() + chrono::Duration::hours(1);
        queue.token_limit_paused = true;
        queue.token_limit_resets_at = Some(reset_time);

        assert!(queue.next_runnable().is_none());
    }

    #[test]
    fn test_check_token_limit_reset_past_time() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.start(id);

        // Set reset time in the past.
        let past = Utc::now() - chrono::Duration::seconds(10);
        queue.pause_for_token_limit(past);
        assert!(queue.token_limit_paused);

        // Should auto-resume.
        let resumed = queue.check_token_limit_reset();
        assert!(resumed);
        assert!(!queue.token_limit_paused);
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Queued);
    }

    #[test]
    fn test_check_token_limit_reset_future_time() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.start(id);

        let future = Utc::now() + chrono::Duration::hours(1);
        queue.pause_for_token_limit(future);

        let resumed = queue.check_token_limit_reset();
        assert!(!resumed);
        assert!(queue.token_limit_paused);
        assert_eq!(
            queue.get(id).unwrap().status,
            QueueEntryStatus::PausedTokenLimit
        );
    }

    #[test]
    fn test_priority_ordering_multiple() {
        let mut queue = AgentQueue::new();
        let low = queue.submit("Low".into(), None, 0);
        let high = queue.submit("High".into(), None, 100);
        let mid = queue.submit("Mid".into(), None, 50);

        let entries: Vec<_> = queue.list().iter().map(|e| e.id).collect();
        assert_eq!(entries, vec![high, mid, low]);

        // next_runnable should return highest priority.
        assert_eq!(queue.next_runnable().unwrap().id, high);
    }

    #[test]
    fn test_fail_nonexistent_returns_false() {
        let mut queue = AgentQueue::new();
        assert!(!queue.fail(Uuid::new_v4(), "error".into()));
    }

    #[test]
    fn test_start_nonexistent_returns_false() {
        let mut queue = AgentQueue::new();
        assert!(!queue.start(Uuid::new_v4()));
    }

    #[test]
    fn test_parse_schedule() {
        let schedule = parse_schedule("Mon:09:00,Wed:14:30,Fri:18:00");
        assert_eq!(schedule.len(), 3);
        assert_eq!(schedule[0].day, Weekday::Mon);
        assert_eq!(schedule[0].hour, 9);
        assert_eq!(schedule[0].minute, 0);
        assert_eq!(schedule[1].day, Weekday::Wed);
        assert_eq!(schedule[1].hour, 14);
        assert_eq!(schedule[1].minute, 30);
        assert_eq!(schedule[2].day, Weekday::Fri);
        assert_eq!(schedule[2].hour, 18);
        assert_eq!(schedule[2].minute, 0);
    }

    #[test]
    fn test_parse_schedule_empty() {
        assert!(parse_schedule("").is_empty());
        assert!(parse_schedule("   ").is_empty());
    }

    #[test]
    fn test_parse_schedule_invalid() {
        // Invalid day.
        assert!(parse_schedule("Xyz:09:00").is_empty());
        // Missing minute.
        assert!(parse_schedule("Mon:09").is_empty());
        // Out of range.
        assert!(parse_schedule("Mon:25:00").is_empty());
        assert!(parse_schedule("Mon:09:61").is_empty());
    }

    #[test]
    fn test_should_auto_process_automatic() {
        let queue = AgentQueue::new();
        assert!(queue.should_auto_process(QueueProcessingMode::Automatic, &[]));
    }

    #[test]
    fn test_should_auto_process_manual() {
        let mut queue = AgentQueue::new();
        assert!(!queue.should_auto_process(QueueProcessingMode::Manual, &[]));
        queue.process_all = true;
        assert!(queue.should_auto_process(QueueProcessingMode::Manual, &[]));
    }

    #[test]
    fn test_should_auto_process_scheduled_process_all() {
        let mut queue = AgentQueue::new();
        assert!(!queue.should_auto_process(QueueProcessingMode::Scheduled, &[]));
        queue.process_all = true;
        assert!(queue.should_auto_process(QueueProcessingMode::Scheduled, &[]));
    }

    #[test]
    fn test_completed_entries() {
        let mut queue = AgentQueue::new();
        let id1 = queue.submit("Task A".into(), None, 0);
        let id2 = queue.submit("Task B".into(), None, 0);
        let id3 = queue.submit("Task C".into(), None, 0);

        queue.start(id1);
        queue.complete(id1);
        queue.start(id2);
        queue.fail(id2, "error".into());
        queue.cancel(id3);

        let completed = queue.completed_entries();
        assert_eq!(completed.len(), 3);
    }

    #[test]
    fn test_processing_mode_from_str() {
        assert_eq!(QueueProcessingMode::from_str("automatic"), QueueProcessingMode::Automatic);
        assert_eq!(QueueProcessingMode::from_str("manual"), QueueProcessingMode::Manual);
        assert_eq!(QueueProcessingMode::from_str("scheduled"), QueueProcessingMode::Scheduled);
        assert_eq!(QueueProcessingMode::from_str("AUTOMATIC"), QueueProcessingMode::Automatic);
        assert_eq!(QueueProcessingMode::from_str("unknown"), QueueProcessingMode::Automatic);
    }

    #[test]
    fn test_processing_mode_as_str() {
        assert_eq!(QueueProcessingMode::Automatic.as_str(), "automatic");
        assert_eq!(QueueProcessingMode::Manual.as_str(), "manual");
        assert_eq!(QueueProcessingMode::Scheduled.as_str(), "scheduled");
    }

    #[test]
    fn test_remove_entry() {
        let mut queue = AgentQueue::new();
        let id1 = queue.submit("Task A".into(), None, 0);
        let id2 = queue.submit("Task B".into(), None, 0);

        assert!(queue.remove(id1));
        assert_eq!(queue.list().len(), 1);
        assert!(queue.get(id1).is_none());
        assert!(queue.get(id2).is_some());
    }

    #[test]
    fn test_remove_nonexistent_returns_false() {
        let mut queue = AgentQueue::new();
        assert!(!queue.remove(Uuid::new_v4()));
    }

    #[test]
    fn test_retry_failed_entry() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.start(id);
        queue.fail(id, "some error".into());

        assert!(queue.retry(id));
        let entry = queue.get(id).unwrap();
        assert_eq!(entry.status, QueueEntryStatus::Queued);
        assert!(entry.started_at.is_none());
        assert!(entry.completed_at.is_none());
        assert!(entry.error.is_none());
    }

    #[test]
    fn test_retry_cancelled_entry() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.cancel(id);

        assert!(queue.retry(id));
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Queued);
    }

    #[test]
    fn test_retry_running_entry_returns_false() {
        let mut queue = AgentQueue::new();
        let id = queue.submit("Task A".into(), None, 0);
        queue.start(id);

        assert!(!queue.retry(id));
        assert_eq!(queue.get(id).unwrap().status, QueueEntryStatus::Running);
    }

    #[test]
    fn test_retry_nonexistent_returns_false() {
        let mut queue = AgentQueue::new();
        assert!(!queue.retry(Uuid::new_v4()));
    }

    #[test]
    fn test_depends_on_blocks_execution() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));

        // Phase 1 should be runnable, phase 2 should not.
        assert_eq!(queue.next_runnable().unwrap().id, phase1);

        // Start and complete phase 1.
        queue.start(phase1);
        // While phase 1 is running (not completed), phase 2 still blocked.
        assert!(queue.next_runnable().is_none());

        queue.complete(phase1);
        // Now phase 2 should be runnable.
        assert_eq!(queue.next_runnable().unwrap().id, phase2);
    }

    #[test]
    fn test_depends_on_cascade_failure() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));
        let phase3 = queue.submit_with_depends("Phase 3".into(), None, 0, Some(phase2));

        queue.start(phase1);
        queue.fail(phase1, "build failed".into());

        // Both phase 2 and phase 3 should be failed.
        assert_eq!(queue.get(phase2).unwrap().status, QueueEntryStatus::Failed);
        assert_eq!(queue.get(phase3).unwrap().status, QueueEntryStatus::Failed);
        assert!(queue.get(phase2).unwrap().error.as_deref().unwrap().contains("failed"));
        assert!(queue.get(phase3).unwrap().error.as_deref().unwrap().contains("failed"));
    }

    #[test]
    fn test_depends_on_cascade_cancel() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));

        queue.cancel(phase1);
        assert_eq!(queue.get(phase1).unwrap().status, QueueEntryStatus::Cancelled);
        assert_eq!(queue.get(phase2).unwrap().status, QueueEntryStatus::Failed);
    }

    #[test]
    fn test_depends_on_no_dependency_is_immediately_runnable() {
        let mut queue = AgentQueue::new();
        let id = queue.submit_with_depends("No dep".into(), None, 0, None);
        assert_eq!(queue.next_runnable().unwrap().id, id);
    }

    #[test]
    fn test_depends_on_missing_dependency_blocks() {
        let mut queue = AgentQueue::new();
        let fake_id = Uuid::new_v4();
        let id = queue.submit_with_depends("Depends on ghost".into(), None, 0, Some(fake_id));
        // Dependency doesn't exist in queue, so not satisfied.
        assert!(queue.next_runnable().is_none());
        let _ = id;
    }

    #[test]
    fn test_depends_on_serde_roundtrip() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));

        // Serialize and deserialize the whole queue.
        let json = serde_json::to_string(&queue).unwrap();
        let restored: AgentQueue = serde_json::from_str(&json).unwrap();

        let entry = restored.get(phase2).unwrap();
        assert_eq!(entry.depends_on, Some(phase1));

        // Entry without depends_on should roundtrip as None.
        let entry1 = restored.get(phase1).unwrap();
        assert_eq!(entry1.depends_on, None);
    }

    #[test]
    fn test_depends_on_serde_missing_field_defaults_to_none() {
        // Simulate a QueueEntry serialized before depends_on existed (no field present).
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "content": "old task",
            "workspace_id": null,
            "priority": 0,
            "status": "queued",
            "created_at": "2025-01-01T00:00:00Z",
            "started_at": null,
            "completed_at": null,
            "error": null,
            "tokens_used": {"input_tokens": 0, "output_tokens": 0, "estimated_cost_usd": 0.0, "model": null}
        }"#;
        let entry: QueueEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.depends_on, None);
    }

    #[test]
    fn test_depends_on_priority_does_not_override_dependency() {
        let mut queue = AgentQueue::new();
        let low = queue.submit("Low priority dep".into(), None, 0);
        // High-priority task that depends on low-priority one.
        let high = queue.submit_with_depends("High priority".into(), None, 100, Some(low));

        // Despite higher priority, high should not be runnable before low completes.
        assert_eq!(queue.next_runnable().unwrap().id, low);

        queue.start(low);
        queue.complete(low);
        assert_eq!(queue.next_runnable().unwrap().id, high);
    }

    #[test]
    fn test_depends_on_retry_respects_dependency() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));

        // Fail phase 1 — cascades to phase 2.
        queue.start(phase1);
        queue.fail(phase1, "error".into());
        assert_eq!(queue.get(phase2).unwrap().status, QueueEntryStatus::Failed);

        // Retry phase 2 — it resets to Queued but still depends on phase 1.
        assert!(queue.retry(phase2));
        assert_eq!(queue.get(phase2).unwrap().status, QueueEntryStatus::Queued);
        // Phase 1 is still failed, so phase 2 should not be runnable.
        assert!(queue.next_runnable().is_none());

        // Now retry phase 1 and complete it.
        assert!(queue.retry(phase1));
        queue.start(phase1);
        queue.complete(phase1);
        // Phase 2 should now be runnable.
        assert_eq!(queue.next_runnable().unwrap().id, phase2);
    }

    #[test]
    fn test_depends_on_fan_out_cascade() {
        let mut queue = AgentQueue::new();
        let parent = queue.submit("Parent".into(), None, 0);
        let child_a = queue.submit_with_depends("Child A".into(), None, 0, Some(parent));
        let child_b = queue.submit_with_depends("Child B".into(), None, 0, Some(parent));

        queue.start(parent);
        queue.fail(parent, "boom".into());

        // Both children should be cascade-failed.
        assert_eq!(queue.get(child_a).unwrap().status, QueueEntryStatus::Failed);
        assert_eq!(queue.get(child_b).unwrap().status, QueueEntryStatus::Failed);
    }

    #[test]
    fn test_depends_on_removed_dependency_blocks() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));

        // Remove phase 1 entirely.
        queue.remove(phase1);

        // Phase 2 depends on a now-absent entry — should not be runnable.
        assert!(queue.next_runnable().is_none());
        let _ = phase2;
    }

    #[test]
    fn test_depends_on_field_preserved_in_get() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));

        assert_eq!(queue.get(phase1).unwrap().depends_on, None);
        assert_eq!(queue.get(phase2).unwrap().depends_on, Some(phase1));
    }

    #[test]
    fn test_depends_on_full_three_phase_lifecycle() {
        let mut queue = AgentQueue::new();
        let p1 = queue.submit("Phase 1".into(), None, 0);
        let p2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(p1));
        let p3 = queue.submit_with_depends("Phase 3".into(), None, 0, Some(p2));

        // Only p1 is runnable.
        assert_eq!(queue.next_runnable().unwrap().id, p1);

        // Execute p1.
        queue.start(p1);
        assert!(queue.next_runnable().is_none()); // p2 blocked, p3 blocked
        queue.complete(p1);

        // Now p2 is runnable.
        assert_eq!(queue.next_runnable().unwrap().id, p2);
        queue.start(p2);
        assert!(queue.next_runnable().is_none()); // p3 blocked
        queue.complete(p2);

        // Now p3 is runnable.
        assert_eq!(queue.next_runnable().unwrap().id, p3);
        queue.start(p3);
        queue.complete(p3);

        // All done.
        assert!(queue.next_runnable().is_none());
        assert_eq!(queue.get(p1).unwrap().status, QueueEntryStatus::Completed);
        assert_eq!(queue.get(p2).unwrap().status, QueueEntryStatus::Completed);
        assert_eq!(queue.get(p3).unwrap().status, QueueEntryStatus::Completed);
    }

    #[test]
    fn test_depends_on_cascade_does_not_affect_independent_tasks() {
        let mut queue = AgentQueue::new();
        let phase1 = queue.submit("Phase 1".into(), None, 0);
        let phase2 = queue.submit_with_depends("Phase 2".into(), None, 0, Some(phase1));
        let independent = queue.submit("Independent".into(), None, 0);

        queue.start(phase1);
        queue.fail(phase1, "error".into());

        // Phase 2 should be cascade-failed, but independent should still be queued.
        assert_eq!(queue.get(phase2).unwrap().status, QueueEntryStatus::Failed);
        assert_eq!(queue.get(independent).unwrap().status, QueueEntryStatus::Queued);
        assert_eq!(queue.next_runnable().unwrap().id, independent);
    }

    #[test]
    fn test_queue_sandbox_default_disabled() {
        let queue = AgentQueue::new();
        assert!(!queue.sandbox_policy().enabled);
    }

    #[test]
    fn test_queue_sandbox_mutate() {
        use crate::sandbox::EnforcementLevel;
        let mut queue = AgentQueue::new();
        queue.sandbox_policy_mut().enabled = true;
        queue.sandbox_policy_mut().allow_network = false;
        queue.sandbox_policy_mut().enforcement = EnforcementLevel::Strict;

        assert!(queue.sandbox_policy().enabled);
        assert!(!queue.sandbox_policy().allow_network);
        assert_eq!(queue.sandbox_policy().enforcement, EnforcementLevel::Strict);
    }

    #[test]
    fn test_queue_sandbox_serde_roundtrip() {
        use crate::sandbox::EnforcementLevel;
        let mut queue = AgentQueue::new();
        queue.sandbox_policy_mut().enabled = true;
        queue.sandbox_policy_mut().enforcement = EnforcementLevel::Strict;
        queue.sandbox_policy_mut().allow_network = false;

        let json = serde_json::to_string(&queue).unwrap();
        let restored: AgentQueue = serde_json::from_str(&json).unwrap();

        assert!(restored.sandbox_policy().enabled);
        assert_eq!(restored.sandbox_policy().enforcement, EnforcementLevel::Strict);
        assert!(!restored.sandbox_policy().allow_network);
    }

    #[test]
    fn test_queue_sandbox_serde_missing_field_defaults() {
        // Simulate old serialized AgentQueue without sandbox_policy field.
        let json = r#"{
            "entries": [],
            "token_limit_paused": false,
            "token_limit_resets_at": null
        }"#;
        let queue: AgentQueue = serde_json::from_str(json).unwrap();
        assert!(!queue.sandbox_policy().enabled);
    }
}
