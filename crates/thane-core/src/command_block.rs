use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A "block" representing a single command invocation and its output.
///
/// Blocks are delimited by OSC 133 shell integration marks:
/// - A: prompt start
/// - B: command start (user pressed Enter)
/// - C: command is executing
/// - D: command finished (with optional exit code)
///
/// This enables smart scroll: when an agent is producing output, the user
/// can scroll up through previous blocks without losing their position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandBlock {
    /// The command text (captured between mark B and C).
    pub command: Option<String>,
    /// Exit code (from mark D).
    pub exit_code: Option<i32>,
    /// When the command started executing.
    pub started_at: Option<DateTime<Utc>>,
    /// When the command finished.
    pub finished_at: Option<DateTime<Utc>>,
    /// Whether this block is still running (between C and D).
    pub running: bool,
}

impl CommandBlock {
    pub fn new() -> Self {
        Self {
            command: None,
            exit_code: None,
            started_at: None,
            finished_at: None,
            running: false,
        }
    }

    /// Mark that the command has started executing.
    pub fn mark_executing(&mut self) {
        self.started_at = Some(Utc::now());
        self.running = true;
    }

    /// Mark that the command has finished.
    pub fn mark_finished(&mut self, exit_code: Option<i32>) {
        self.exit_code = exit_code;
        self.finished_at = Some(Utc::now());
        self.running = false;
    }

    /// Duration of execution, if both started and finished.
    pub fn duration(&self) -> Option<chrono::Duration> {
        match (self.started_at, self.finished_at) {
            (Some(start), Some(end)) => Some(end - start),
            _ => None,
        }
    }

    /// Human-readable duration string.
    pub fn duration_display(&self) -> Option<String> {
        let dur = self.duration()?;
        let secs = dur.num_seconds();
        if secs < 60 {
            Some(format!("{secs}s"))
        } else if secs < 3600 {
            Some(format!("{}m{}s", secs / 60, secs % 60))
        } else {
            Some(format!("{}h{}m", secs / 3600, (secs % 3600) / 60))
        }
    }
}

impl Default for CommandBlock {
    fn default() -> Self {
        Self::new()
    }
}

/// OSC 133 shell integration mark types (mirrored from thane-terminal
/// to keep thane-core free of circular dependencies).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellMark {
    /// Prompt start (A).
    PromptStart,
    /// Command start (B) — user is typing.
    CommandStart,
    /// Command executed (C) — user pressed Enter.
    CommandExecuted,
    /// Command finished (D) with optional exit code.
    CommandFinished(Option<i32>),
}

/// Tracks command blocks for a terminal panel using OSC 133 shell marks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockTracker {
    /// Completed blocks in chronological order.
    blocks: Vec<CommandBlock>,
    /// The current in-progress block (between prompt start and command finish).
    current: Option<CommandBlock>,
    /// Whether output is actively being produced (between C and D marks).
    pub output_active: bool,
    /// Maximum blocks to keep.
    max_blocks: usize,
}

impl BlockTracker {
    pub fn new(max_blocks: usize) -> Self {
        Self {
            blocks: Vec::new(),
            current: None,
            output_active: false,
            max_blocks,
        }
    }

    /// Handle a shell integration mark event.
    pub fn handle_mark(&mut self, mark: ShellMark) {
        match mark {
            ShellMark::PromptStart => {
                // Flush any previous block.
                if let Some(block) = self.current.take() {
                    self.push_block(block);
                }
                self.current = Some(CommandBlock::new());
                self.output_active = false;
            }
            ShellMark::CommandStart => {
                // User has started typing — nothing to do yet.
            }
            ShellMark::CommandExecuted => {
                if let Some(ref mut block) = self.current {
                    block.mark_executing();
                }
                self.output_active = true;
            }
            ShellMark::CommandFinished(exit_code) => {
                if let Some(ref mut block) = self.current {
                    block.mark_finished(exit_code);
                }
                self.output_active = false;
            }
        }
    }

    /// Set the command text for the current block.
    pub fn set_command(&mut self, command: String) {
        if let Some(ref mut block) = self.current {
            block.command = Some(command);
        }
    }

    fn push_block(&mut self, block: CommandBlock) {
        self.blocks.push(block);
        while self.blocks.len() > self.max_blocks {
            self.blocks.remove(0);
        }
    }

    /// Get all completed blocks.
    pub fn blocks(&self) -> &[CommandBlock] {
        &self.blocks
    }

    /// Get the current in-progress block.
    pub fn current_block(&self) -> Option<&CommandBlock> {
        self.current.as_ref()
    }

    /// Total block count (completed + current).
    pub fn count(&self) -> usize {
        self.blocks.len() + if self.current.is_some() { 1 } else { 0 }
    }

    /// Get the last N completed blocks.
    pub fn recent(&self, n: usize) -> &[CommandBlock] {
        let start = self.blocks.len().saturating_sub(n);
        &self.blocks[start..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_lifecycle() {
        let mut tracker = BlockTracker::new(100);

        // Prompt appears.
        tracker.handle_mark(ShellMark::PromptStart);
        assert!(tracker.current_block().is_some());
        assert!(!tracker.output_active);

        // User types command and presses Enter.
        tracker.set_command("ls -la".to_string());
        tracker.handle_mark(ShellMark::CommandStart);
        tracker.handle_mark(ShellMark::CommandExecuted);
        assert!(tracker.output_active);
        assert!(tracker.current_block().unwrap().running);

        // Command finishes.
        tracker.handle_mark(ShellMark::CommandFinished(Some(0)));
        assert!(!tracker.output_active);
        assert!(!tracker.current_block().unwrap().running);
        assert_eq!(tracker.current_block().unwrap().exit_code, Some(0));

        // New prompt — previous block is pushed to completed.
        tracker.handle_mark(ShellMark::PromptStart);
        assert_eq!(tracker.blocks().len(), 1);
        assert_eq!(tracker.blocks()[0].command.as_deref(), Some("ls -la"));
        assert_eq!(tracker.blocks()[0].exit_code, Some(0));
    }

    #[test]
    fn test_block_max_limit() {
        let mut tracker = BlockTracker::new(3);

        for i in 0..5 {
            tracker.handle_mark(ShellMark::PromptStart);
            tracker.set_command(format!("cmd{i}"));
            tracker.handle_mark(ShellMark::CommandExecuted);
            tracker.handle_mark(ShellMark::CommandFinished(Some(0)));
        }
        // Final prompt start flushes the 5th block.
        tracker.handle_mark(ShellMark::PromptStart);

        assert_eq!(tracker.blocks().len(), 3);
        assert_eq!(tracker.blocks()[0].command.as_deref(), Some("cmd2"));
        assert_eq!(tracker.blocks()[2].command.as_deref(), Some("cmd4"));
    }

    #[test]
    fn test_duration_display() {
        let mut block = CommandBlock::new();
        block.mark_executing();
        std::thread::sleep(std::time::Duration::from_millis(10));
        block.mark_finished(Some(0));

        // Duration should be >=0s.
        let display = block.duration_display().unwrap();
        assert!(display.ends_with('s'));
    }

    #[test]
    fn test_recent_blocks() {
        let mut tracker = BlockTracker::new(100);

        for i in 0..10 {
            tracker.handle_mark(ShellMark::PromptStart);
            tracker.set_command(format!("cmd{i}"));
            tracker.handle_mark(ShellMark::CommandExecuted);
            tracker.handle_mark(ShellMark::CommandFinished(Some(0)));
        }
        tracker.handle_mark(ShellMark::PromptStart);

        let recent = tracker.recent(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].command.as_deref(), Some("cmd7"));
    }
}
