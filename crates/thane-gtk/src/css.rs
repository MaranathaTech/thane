use gtk4::{CssProvider, gdk};

/// Default CSS for thane UI components.
///
/// Color palette aligned with the marketing site:
///   --bg-primary:      #0c0c0e  (terminal / deepest background)
///   --bg-surface:      #141416  (panels, sidebar, header)
///   --bg-raised:       #1a1a1d  (hover states, active items)
///   --border:          #23232a  (subtle dividers)
///   --border-emphasis: #2d2d35  (focused / active borders)
///   --text-primary:    #e4e4e7  (body text)
///   --text-secondary:  #a1a1aa  (labels, metadata)
///   --text-muted:      #71717a  (de-emphasized)
///   --accent:          #818cf8  (indigo – brand color)
///   --success:         #4ade80
///   --warning:         #fbbf24
///   --error:           #f87171
///
/// Base UI font size: 14px (configurable via settings slider).
/// Typography scale: 12 / 13 / 14 / 15 / 16 / 30
const DEFAULT_CSS: &str = r#"
/* thane default stylesheet — refined dark theme */

/* ──────────────────────────────────────────────
   Header bar
   ────────────────────────────────────────────── */
.thane-header {
    background-color: #141416;
    color: #e4e4e7;
    border-bottom: 1px solid #23232a;
    min-height: 44px;
}
.thane-header-title {
    font-weight: 600;
    font-size: 16px;
    color: #e4e4e7;
}

/* ──────────────────────────────────────────────
   Sidebar
   ────────────────────────────────────────────── */
.sidebar {
    background-color: #141416;
    color: #e4e4e7;
    border-right: 1px solid #23232a;
    min-width: 240px;
}

.sidebar-row {
    padding: 10px 12px;
    border-radius: 6px;
    margin: 2px 6px;
    transition: background-color 150ms ease-in-out;
}

.sidebar-row:selected {
    background-color: alpha(#818cf8, 0.15);
}

.sidebar-row:hover {
    background-color: #1a1a1d;
}

.sidebar-row-selected {
    background-color: alpha(#818cf8, 0.15);
}

.workspace-close-btn {
    min-width: 16px;
    min-height: 16px;
    padding: 0;
    opacity: 0.0;
    transition: opacity 150ms ease-in-out;
}

.sidebar-row:hover .workspace-close-btn {
    opacity: 0.4;
}

.workspace-close-btn:hover {
    opacity: 1.0;
    color: #f87171;
}

.workspace-title {
    font-weight: 600;
    font-size: 15px;
    color: #e4e4e7;
}

.workspace-cwd {
    font-size: 13px;
    color: #71717a;
}

.workspace-last-prompt {
    font-size: 12px;
    font-style: italic;
    color: #a78bfa;
}

.workspace-cost {
    font-size: 13px;
    font-family: monospace;
    color: #71717a;
}

.workspace-git {
    font-size: 13px;
    color: #4ade80;
}

.workspace-git-dirty {
    color: #fbbf24;
}

.workspace-git-untracked {
    font-size: 13px;
    color: #71717a;
    font-style: italic;
}

.workspace-git-untracked-icon {
    color: #71717a;
    opacity: 0.6;
}

.workspace-agent-active {
    font-size: 13px;
    color: #4ade80;
}

.workspace-agent-stalled {
    font-size: 13px;
    color: #fbbf24;
}

.workspace-tag {
    font-size: 12px;
    color: #818cf8;
    background-color: alpha(#818cf8, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
}

/* Notification ring animation */
@keyframes notification-ring {
    0%   { box-shadow: 0 0 0 0 alpha(#818cf8, 0.7); }
    70%  { box-shadow: 0 0 0 6px alpha(#818cf8, 0); }
    100% { box-shadow: 0 0 0 0 alpha(#818cf8, 0); }
}

.notification-ring {
    animation: notification-ring 1.5s ease infinite;
}

/* Unread badge */
.unread-badge {
    background-color: #818cf8;
    color: white;
    border-radius: 10px;
    padding: 1px 6px;
    font-size: 12px;
    font-weight: bold;
    min-width: 16px;
}

/* ──────────────────────────────────────────────
   Terminal pane
   ────────────────────────────────────────────── */
.terminal-pane {
    background-color: #0c0c0e;
    border: 1px solid #0c0c0e;
}

.terminal-pane:focus-within {
    border: 1px solid alpha(#818cf8, 0.4);
}

/* Focused pane ring (multi-pane only) */
.pane-focused {
    border: 2px solid alpha(#818cf8, 0.6);
    border-radius: 2px;
}

/* Unfocused pane dimming */
.pane-unfocused {
    opacity: 0.90;
}

/* ──────────────────────────────────────────────
   Browser omnibar
   ────────────────────────────────────────────── */
.omnibar {
    padding: 6px 12px;
    background-color: #141416;
    border-bottom: 1px solid #23232a;
}

.omnibar entry {
    border-radius: 16px;
    padding: 4px 12px;
    background-color: #1a1a1d;
    color: #e4e4e7;
}

/* ──────────────────────────────────────────────
   Agent status (global)
   ────────────────────────────────────────────── */
.agent-active {
    color: #4ade80;
}

.agent-stalled {
    color: #fbbf24;
}

.agent-inactive {
    color: #71717a;
}

/* Cost tracker */
.cost-display {
    font-family: monospace;
    font-size: 14px;
    color: #a1a1aa;
}

/* ──────────────────────────────────────────────
   Notification panel
   ────────────────────────────────────────────── */
.notification-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
    min-width: 320px;
}

.notification-item {
    padding: 12px 14px;
    border-bottom: 1px solid #23232a;
    transition: background-color 150ms ease-in-out;
}

.notification-item:hover {
    background-color: #1a1a1d;
}

.notification-item-unread {
    background-color: alpha(#818cf8, 0.06);
    border-left: 3px solid #818cf8;
}

.notification-title {
    font-weight: 600;
    font-size: 15px;
    color: #e4e4e7;
}

.notification-body {
    font-size: 14px;
    color: #a1a1aa;
}

.notification-time {
    font-size: 13px;
    color: #71717a;
}

/* Notification urgency badges */
.notification-urgency-low {
    font-size: 12px;
    color: #71717a;
    background-color: alpha(#71717a, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
}
.notification-urgency-normal {
    font-size: 12px;
    color: #818cf8;
    background-color: alpha(#818cf8, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
}
.notification-urgency-critical {
    font-size: 12px;
    color: #f87171;
    background-color: alpha(#f87171, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
}

/* Port indicator */
.port-badge {
    background-color: alpha(#818cf8, 0.12);
    color: #818cf8;
    border-radius: 4px;
    padding: 1px 6px;
    font-size: 12px;
    font-family: monospace;
    transition: background-color 150ms ease-in-out;
}

.port-badge:hover {
    background-color: alpha(#818cf8, 0.25);
}

/* ──────────────────────────────────────────────
   Audit panel
   ────────────────────────────────────────────── */
.audit-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
    min-width: 360px;
}

.audit-item {
    padding: 10px 14px;
    border-bottom: 1px solid #23232a;
}

.audit-item-critical {
    background-color: alpha(#f87171, 0.08);
    border-left: 3px solid #f87171;
}

.audit-item-alert {
    background-color: alpha(#fbbf24, 0.05);
    border-left: 3px solid #fbbf24;
}

.audit-item-warning {
    border-left: 3px solid alpha(#fbbf24, 0.5);
}

.audit-severity {
    font-size: 12px;
    font-weight: bold;
    border-radius: 3px;
    padding: 1px 5px;
}

.audit-severity-info {
    color: #a1a1aa;
    background-color: alpha(#a1a1aa, 0.10);
}

.audit-severity-warning {
    color: #fbbf24;
    background-color: alpha(#fbbf24, 0.12);
}

.audit-severity-alert {
    color: #fbbf24;
    background-color: alpha(#fbbf24, 0.18);
}

.audit-severity-critical {
    color: #f87171;
    background-color: alpha(#f87171, 0.18);
}

.audit-event-type {
    font-size: 14px;
    font-weight: 600;
    color: #e4e4e7;
}

.audit-description {
    font-size: 14px;
    color: #a1a1aa;
}

.audit-agent-badge {
    font-size: 12px;
    color: #818cf8;
    background-color: alpha(#818cf8, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
}

.audit-filter-btn {
    font-size: 13px;
    padding: 3px 8px;
    min-height: 24px;
    border-radius: 4px;
}

/* Sandbox badge */
.sandbox-badge {
    background-color: alpha(#fbbf24, 0.12);
    color: #fbbf24;
    border-radius: 4px;
    padding: 1px 6px;
    font-size: 12px;
    font-weight: bold;
}

/* ──────────────────────────────────────────────
   Git diff panel
   ────────────────────────────────────────────── */
.git-diff-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
    min-width: 420px;
}

.git-diff-status {
    font-size: 14px;
    color: #71717a;
}

.git-diff-subtitle {
    font-size: 13px;
    color: #71717a;
    font-family: monospace;
}

.git-diff-file {
    border-bottom: 1px solid alpha(#23232a, 0.6);
    padding-bottom: 2px;
}

.git-diff-file-expanded {
    border-left: 2px solid #818cf8;
}

.git-diff-file-row {
    padding: 6px 4px;
    border-radius: 4px;
    transition: background-color 150ms ease-in-out;
}

.git-diff-file-row:hover {
    background-color: #1a1a1d;
}

.git-diff-status-badge {
    font-size: 12px;
    font-weight: bold;
    font-family: monospace;
    border-radius: 3px;
    padding: 1px 5px;
    min-width: 16px;
}

.git-diff-modified {
    color: #fbbf24;
    background-color: alpha(#fbbf24, 0.12);
}

.git-diff-added {
    color: #4ade80;
    background-color: alpha(#4ade80, 0.12);
}

.git-diff-deleted {
    color: #f87171;
    background-color: alpha(#f87171, 0.12);
}

.git-diff-renamed {
    color: #818cf8;
    background-color: alpha(#818cf8, 0.12);
}

.git-diff-filename {
    font-size: 14px;
    font-weight: 600;
    color: #e4e4e7;
}

.git-diff-dirname {
    font-size: 13px;
    color: #71717a;
}

.git-diff-dir-row {
    padding: 6px 4px;
    border-radius: 4px;
    transition: background-color 150ms ease-in-out;
}

.git-diff-dir-row:hover {
    background-color: #1a1a1d;
}

.git-diff-dir-name {
    font-size: 14px;
    font-weight: 600;
    color: #a1a1aa;
}

.git-diff-dir-collapsed {
    opacity: 0.7;
}

.git-diff-chevron {
    font-size: 10px;
    color: #71717a;
    min-width: 14px;
}

.git-diff-line-count-plus {
    font-size: 13px;
    font-family: monospace;
    color: #4ade80;
}

.git-diff-line-count-minus {
    font-size: 13px;
    font-family: monospace;
    color: #f87171;
}

.git-diff-hunk-header {
    font-size: 13px;
    font-family: monospace;
    color: #818cf8;
    background-color: alpha(#818cf8, 0.06);
    padding: 3px 6px;
}

.git-diff-line {
    font-size: 13px;
    font-family: monospace;
    padding: 0px 2px;
}

.git-diff-line-added {
    background-color: alpha(#4ade80, 0.10);
    color: #e4e4e7;
}

.git-diff-line-removed {
    background-color: alpha(#f87171, 0.10);
    color: #e4e4e7;
}

.git-diff-line-context {
    color: #a1a1aa;
}

.git-diff-gutter {
    font-size: 12px;
    font-family: monospace;
    color: #52525b;
    min-width: 32px;
    padding-right: 4px;
}

.git-diff-prefix {
    font-size: 13px;
    font-family: monospace;
    color: #71717a;
}

.git-diff-content {
    font-size: 13px;
    font-family: monospace;
}

/* ──────────────────────────────────────────────
   Jump-to-bottom button (terminal overlay)
   ────────────────────────────────────────────── */
.jump-to-bottom {
    background-color: #818cf8;
    color: white;
    border-radius: 16px;
    padding: 5px 18px;
    font-size: 14px;
    font-weight: 600;
    box-shadow: 0 2px 12px rgba(0, 0, 0, 0.5);
}

/* ──────────────────────────────────────────────
   Settings panel
   ────────────────────────────────────────────── */
.settings-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
    min-width: 320px;
}

.settings-panel label {
    color: #e4e4e7;
    font-size: 15px;
}

.settings-hint {
    font-size: 13px;
    color: #71717a;
    font-style: italic;
}

.settings-section-title {
    font-size: 13px;
    font-weight: 600;
    color: #818cf8;
    letter-spacing: 0.5px;
}

/* ──────────────────────────────────────────────
   Token usage panel
   ────────────────────────────────────────────── */
.token-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
    min-width: 320px;
}

.token-cost-large {
    font-size: 30px;
    font-weight: bold;
    font-family: monospace;
    color: #e4e4e7;
}

.token-total {
    font-size: 15px;
    font-family: monospace;
    color: #71717a;
}

.token-detail-label {
    font-size: 14px;
    color: #a1a1aa;
}

.token-detail-value {
    font-size: 14px;
    font-family: monospace;
    color: #e4e4e7;
}

.token-section-header {
    font-size: 12px;
    font-weight: 600;
    color: #71717a;
    letter-spacing: 1px;
}

.token-limit-bar {
    min-height: 8px;
    border-radius: 4px;
}

.token-limit-bar trough {
    background-color: #1a1a1d;
    border-radius: 4px;
    min-height: 8px;
}

.token-limit-bar progress {
    background-color: #818cf8;
    border-radius: 4px;
    min-height: 8px;
}

.workspace-cost-historical {
    font-style: italic;
    opacity: 0.5;
}

.token-no-caps {
    font-style: italic;
    color: #71717a;
    font-size: 14px;
}

/* ──────────────────────────────────────────────
   Status bar
   ────────────────────────────────────────────── */
.status-bar {
    background-color: #141416;
    color: #e4e4e7;
    border-top: 1px solid #23232a;
    min-height: 28px;
    padding: 0px 12px;
}

.status-bar label {
    font-size: 14px;
    color: #a1a1aa;
}

.status-bar-clickable {
    transition: background-color 150ms ease-in-out;
}

.status-bar-clickable:hover {
    background-color: #1a1a1d;
    border-radius: 4px;
}

.status-agent-active {
    color: #4ade80;
    font-weight: 600;
}

.status-agent-inactive {
    color: #71717a;
}

/* ──────────────────────────────────────────────
   Tab bar (per-pane panel tabs)
   ────────────────────────────────────────────── */
.tab-bar {
    background-color: #141416;
    border-bottom: 1px solid #23232a;
    min-height: 34px;
    padding: 0 4px;
}

.tab-item {
    padding: 6px 14px;
    border-radius: 6px 6px 0 0;
    margin-top: 2px;
    color: #a1a1aa;
    transition: background-color 150ms ease-in-out, color 150ms ease-in-out;
}

.tab-item:hover {
    background-color: #1a1a1d;
    color: #e4e4e7;
}

.tab-item-selected {
    background-color: #0c0c0e;
    color: #e4e4e7;
    border-bottom: 2px solid #818cf8;
}

.tab-icon {
    opacity: 0.6;
}

.tab-item-selected .tab-icon {
    opacity: 1.0;
}

.tab-title {
    font-size: 15px;
    font-weight: 500;
}

.tab-close-btn {
    min-width: 16px;
    min-height: 16px;
    padding: 0;
    margin-left: 6px;
    opacity: 0.3;
    transition: opacity 150ms ease-in-out;
}

.tab-close-btn:hover {
    opacity: 1.0;
    color: #f87171;
}

.tab-bar-actions-left {
    padding: 0 2px;
}

.tab-bar-actions {
    margin-left: auto;
    padding: 0 2px;
}

.tab-action-btn {
    min-width: 24px;
    min-height: 24px;
    padding: 0;
    margin: 4px 1px;
    opacity: 0.4;
    transition: opacity 150ms ease-in-out;
}

.tab-action-btn:hover {
    opacity: 1.0;
    color: #818cf8;
}

.tab-close-pane-btn:hover {
    color: #f87171;
}

/* ──────────────────────────────────────────────
   Sidebar collapsed (compact) mode
   ────────────────────────────────────────────── */
.sidebar-collapsed {
    min-width: 48px;
}

.sidebar-avatar {
    min-width: 32px;
    min-height: 32px;
    border-radius: 16px;
    font-size: 12px;
    font-weight: bold;
    background-color: #1a1a1d;
    color: #a1a1aa;
    transition: background-color 150ms ease-in-out;
}

.sidebar-avatar:hover {
    background-color: #23232a;
}

.sidebar-avatar-selected {
    background-color: alpha(#818cf8, 0.25);
    border: 1px solid alpha(#818cf8, 0.6);
    color: #e4e4e7;
}

.sidebar-avatar-sandboxed {
    background-color: alpha(#f59e0b, 0.2);
    border: 1px solid alpha(#f59e0b, 0.6);
    color: #fbbf24;
}

.sidebar-avatar-sandboxed.sidebar-avatar-selected {
    background-color: alpha(#f59e0b, 0.3);
    border: 1px solid alpha(#f59e0b, 0.8);
    color: #fcd34d;
}

/* ──────────────────────────────────────────────
   Terminal search bar
   ────────────────────────────────────────────── */
.terminal-search-bar {
    background-color: #141416;
    border-bottom: 1px solid #23232a;
    padding: 6px 8px;
}

.terminal-search-bar entry {
    background-color: #1a1a1d;
    color: #e4e4e7;
    border-radius: 6px;
    padding: 4px 10px;
}

/* ──────────────────────────────────────────────
   Help panel
   ────────────────────────────────────────────── */
.help-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
    min-width: 320px;
}

.help-section-title {
    font-size: 13px;
    font-weight: bold;
    color: #818cf8;
    letter-spacing: 0.5px;
}

.help-shortcut-key {
    font-size: 13px;
    font-family: monospace;
    background-color: #1a1a1d;
    border-radius: 4px;
    padding: 2px 6px;
    min-width: 80px;
}

.help-shortcut-desc {
    font-size: 14px;
    color: #a1a1aa;
}

.help-tip {
    font-size: 14px;
    color: #a1a1aa;
    padding-left: 8px;
}

/* ──────────────────────────────────────────────
   Status entries (workspace sidebar)
   ────────────────────────────────────────────── */
.status-entry-label {
    font-size: 12px;
    color: #71717a;
}

.status-entry-normal {
    font-size: 12px;
    color: #a1a1aa;
}

.status-entry-success {
    font-size: 12px;
    color: #4ade80;
}

.status-entry-warning {
    font-size: 12px;
    color: #fbbf24;
}

.status-entry-error {
    font-size: 12px;
    color: #f87171;
}

.status-entry-muted {
    font-size: 12px;
    color: #52525b;
}

/* ──────────────────────────────────────────────
   Command block status (status bar)
   ────────────────────────────────────────────── */
.status-leader-badge {
    font-size: 12px;
    font-weight: bold;
    color: #e4e4e7;
    background-color: #818cf8;
    border-radius: 4px;
    padding: 1px 8px;
    margin: 0 4px;
}

.status-cmd-ok {
    color: #4ade80;
}

.status-cmd-fail {
    color: #f87171;
}

.status-cmd-info {
    font-size: 13px;
    font-family: monospace;
}

/* ──────────────────────────────────────────────
   Agent queue panel
   ────────────────────────────────────────────── */
.agent-queue-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
}

.queue-item {
    padding: 10px 14px;
    border-bottom: 1px solid #23232a;
}

.queue-content {
    font-size: 14px;
    color: #e4e4e7;
}

.panel-meta {
    font-size: 12px;
    color: #71717a;
}

.queue-status-queued {
    font-size: 12px;
    color: #818cf8;
    background-color: alpha(#818cf8, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
    font-weight: bold;
}

.queue-status-running {
    font-size: 12px;
    color: #4ade80;
    background-color: alpha(#4ade80, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
    font-weight: bold;
}

.queue-status-completed {
    font-size: 12px;
    color: #4ade80;
}

.queue-status-failed {
    font-size: 12px;
    color: #f87171;
}

.queue-status-cancelled {
    font-size: 12px;
    color: #71717a;
}

.queue-status-paused {
    font-size: 12px;
    color: #fbbf24;
    background-color: alpha(#fbbf24, 0.12);
    border-radius: 4px;
    padding: 1px 6px;
    font-weight: bold;
}

.queue-hint {
    background-color: alpha(#818cf8, 0.06);
    border-left: 3px solid alpha(#818cf8, 0.4);
    padding: 10px 12px;
    margin: 8px 12px;
    border-radius: 4px;
}

.queue-hint-text {
    font-size: 14px;
    color: #a1a1aa;
}

.queue-hint-example {
    font-size: 13px;
    color: #71717a;
    font-style: italic;
}

.claude-missing-banner {
    background-color: alpha(#fbbf24, 0.08);
    border-left: 3px solid #fbbf24;
    padding: 10px 12px;
    margin: 8px 12px;
    border-radius: 4px;
}

.claude-missing-text {
    font-size: 13px;
    color: #fbbf24;
}

.queue-empty-hint {
    font-size: 13px;
    color: #71717a;
}

.queue-process-bar {
    border-top: 1px solid #23232a;
}

.queue-paused-banner {
    background-color: alpha(#fbbf24, 0.08);
    border-left: 3px solid #fbbf24;
    padding: 10px 12px;
    margin: 8px 12px;
    border-radius: 4px;
}

.queue-paused-text {
    font-size: 14px;
    color: #fbbf24;
    font-weight: 600;
}

.queue-paused-countdown {
    font-size: 13px;
    color: #a1a1aa;
}

.status-queue-paused {
    color: #fbbf24;
    font-weight: 600;
}

/* Utilization color thresholds for cost display */
.utilization-ok {
    color: #a3e635;
}

.utilization-warn {
    color: #fbbf24;
}

.utilization-critical {
    color: #f87171;
    font-weight: 600;
}

/* ──────────────────────────────────────────────
   Sandbox configuration panel
   ────────────────────────────────────────────── */
.sandbox-panel {
    background-color: #141416;
    color: #e4e4e7;
    border-left: 1px solid #23232a;
}

.sandbox-section-title {
    font-size: 13px;
    color: #71717a;
    font-weight: 600;
    margin-top: 12px;
}

.sandbox-path-item {
    font-size: 13px;
    font-family: monospace;
    color: #a1a1aa;
}

.sandbox-path-remove:hover {
    color: #f87171;
}

/* ──────────────────────────────────────────────
   Modal dialogs (dark theme matching main window)
   ────────────────────────────────────────────── */
/* ──────────────────────────────────────────────
   Modal dialogs
   ────────────────────────────────────────────── */
.thane-dialog-content {
    background-color: #141416;
    color: #e4e4e7;
}

.thane-dialog-content label {
    color: #e4e4e7;
}

.thane-dialog-content .dim-label {
    color: #a1a1aa;
}

.thane-dialog-content entry {
    background-color: #0c0c0e;
    color: #e4e4e7;
}

/* ──────────────────────────────────────────────
   Recently closed history (sidebar)
   ────────────────────────────────────────────── */
.history-section-header {
    font-size: 13px;
    font-weight: 600;
    color: #71717a;
}

.history-row {
    padding: 6px 4px;
    border-radius: 6px;
    transition: background-color 150ms ease-in-out;
}

.history-row:hover {
    background-color: #1a1a1d;
}

.history-title {
    font-size: 14px;
    color: #a1a1aa;
}

.history-time {
    font-size: 12px;
    color: #52525b;
}

/* ──────────────────────────────────────────────
   Audit event detail modal
   ────────────────────────────────────────────── */
.audit-detail-field-label {
    font-weight: bold;
    font-size: 12px;
    color: #a1a1aa;
}

.audit-detail-metadata {
    font-family: 'JetBrains Mono NL', monospace;
    font-size: 13px;
    background-color: #0c0c0e;
    color: #e4e4e7;
    padding: 8px;
    border-radius: 6px;
}
"#;

/// Load the application CSS.
pub fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_string(DEFAULT_CSS);

    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
