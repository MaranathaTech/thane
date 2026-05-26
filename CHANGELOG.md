# Changelog

All notable changes to thane are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

When adding an entry for the next release, copy the **Template** at the
bottom of this file into a new section under `[Unreleased]`.

---

## [Unreleased]

---

## [0.1.0-beta.22] — 2026-05-25

### Added

- **Enterprise audit logging.** Every audit event now carries the OS
  user (`system_user` / `system_uid`), is signed with HMAC-SHA256
  alongside the SHA-256 hash chain, and (for rotated log files) is
  encrypted at rest with AES-256-GCM. Keys live in macOS Keychain /
  Linux Secret Service with HKDF-derived sub-keys.
- **Offline audit verification.** New `thane-cli audit verify` checks
  chain + signatures against a log directory. `thane-cli audit
  export-key` / `import-key` let an auditor take the verification key
  offline.
- **PII and secret redaction.** Configurable policy
  (`audit-redaction-policy` = `none` / `redact` / `strict`) scrubs
  emails, SSNs, credit-card numbers (with Luhn check), and bearer
  tokens for Anthropic, OpenAI, GitHub, AWS, Slack, and JWTs before
  signing and persisting.
- **Time-based retention.** `audit-retention-days` (default 90)
  purges rotated audit files past the window. UI labels now reflect
  the configured value instead of a hardcoded "7 days".
- **Restricted log clearing.** `audit.clear` is gated behind
  `audit-allow-clear` (default false) and requires a reason that lands
  in the `AuditCleared` marker event.
- **External log shipping (new `thane-audit-sink` crate).** Sink
  framework with bounded in-memory queue, batched dispatch,
  exponential-backoff retries, dead-letter file, and per-sink severity
  + event-type filters. Six sinks shipping:
  - **syslog** (RFC 5424 over TCP, optional TLS, octet-counting framing)
  - **webhook** (HMAC-signed `X-Thane-Signature: t=<ts>,v1=<hex>`)
  - **Amazon S3** (gzipped JSONL, SSE-S3 / SSE-KMS, optional Object
    Lock for compliance retention, works against R2 / MinIO too)
  - **Splunk HEC** (newline-delimited HEC events, configurable index)
  - **Datadog Logs** (regional intake, configurable env / service /
    ddtags)
  - **Grafana Loki** (multi-tenant via `X-Scope-OrgID`, low-cardinality
    stream labels, optional gzip, bearer / basic / mTLS auth)
- **Enterprise policy override.** MDM-deployed `policy.json` at
  `/Library/Application Support/thane/policy.json` (macOS) or
  `/etc/thane/policy.json` (Linux) locks audit settings the end user
  cannot disable. Locked-key indicators show in the settings UI with
  an `issued_by` banner. macOS additionally honors Managed Preferences
  (`/Library/Managed Preferences/com.thane.app.plist`).
- **Self-service enrollment.** `thane-cli enterprise enroll <url>`,
  `enterprise status`, `enterprise leave` for orgs deploying without
  full MDM.
- **Daemon at login (new `thane-daemon` crate).** Runs the IPC server
  and queue executor without the GUI. macOS LaunchAgent installer
  (`thane-daemon --install-launch-agent`) writes
  `~/Library/LaunchAgents/com.thane.daemon.plist`. Linux equivalent
  installs a systemd user unit
  (`~/.config/systemd/user/thane-daemon.service`). GUI app detects the
  external daemon and yields IPC / executor ownership to it.
- **New CLI:** `thane-cli daemon start|stop|restart`, `thane-cli
  system status` (daemon health + uptime + version), `thane-cli audit
  dlq list|retry|clear` for inspecting the sink dead-letter queue.
- **Queue execution prompt capture.** Headless `claude --print` queue
  tasks now emit a `UserPrompt` audit event with the full prompt text
  (gated by `audit-queue-prompts`, default on), matching the
  interactive Claude Code session capture.
- **Compliance + admin docs.** New
  [`dist/public/AUDIT_LOG.md`](dist/public/AUDIT_LOG.md) (end-to-end
  audit pipeline reference) plus expansions in
  [`COMPLIANCE.md`](dist/public/COMPLIANCE.md),
  [`API.md`](dist/public/API.md), and
  [`ENTERPRISE.md`](dist/public/ENTERPRISE.md) (Kandji / Intune /
  Munki / Jamf / Ansible MDM profiles, daemon-at-login operational
  guide, decommissioning playbook, longer-form Loki + Grafana setup
  recipe with sample LogQL queries).
- `CHANGELOG.md` at the repo root following the Keep a Changelog
  format.

### Changed

- `~/.claude/CLAUDE.md` global instructions template bumped to v5:
  dependency chaining (`thane-cli queue submit --depends-on`) is now
  opt-in. Default is independent phase submission, reversing the v4
  behavior that auto-chained every queued phase. Bridge now injects
  these instructions at startup on macOS (previously GTK-only).
- Audit event schema: added `system_user`, `system_uid`, and `hmac`
  fields. The hash chain now links event-N's `prev_hash` to the
  *signed* HMAC of event-N-1 rather than the raw JSON, so an attacker
  without the HMAC key cannot rewrite the chain.
- `thane-cli queue cancel` / `queue status`: canonical parameter name
  is now `entry_id`. The macOS RPC handler historically accepted only
  `id`; both names are accepted now but only `entry_id` is documented.

### Fixed

- macOS: `thane-cli queue cancel` and `queue status` now work
  end-to-end. The macOS RPC handler previously expected `id` while
  the CLI sent `entry_id`, producing `Missing 'id' parameter`.
- macOS: queue executor now terminates the running subprocess when an
  entry is cancelled. Previously the cancel only flipped the status
  flag, leaving the `claude` subprocess running until it finished on
  its own.
- Linux: same kill-on-cancel fix in the GTK frontend's queue poll loop.
- macOS: `claude` binary now resolved via comprehensive path search
  (`~/.local/bin`, NVM versions, Homebrew paths, `~/.cargo/bin`, plus a
  `zsh -lc 'command -v claude'` fallback). Fixes Exit 127 failures
  when launchd's minimal PATH did not include the user's tool install
  location. Configurable override via `agent-claude-path`.
- macOS: spawned queue subprocesses now receive an augmented PATH
  covering Homebrew (Intel + Apple Silicon), `/usr/local`,
  `~/.local/bin`, `~/.cargo/bin`, NVM, etc.
- macOS: bridge now injects `~/.claude/CLAUDE.md` instructions at
  startup. Previously only the GTK frontend did this, so macOS users
  got stale or missing instructions.

### Removed

- Dead Swift duplicate of the CLAUDE.md injector in `AppDelegate.swift`
  (was using a stale v3 marker; the canonical injector is
  `crates/thane-platform/src/claude_md.rs`, now wired into both the
  GTK setup path and the macOS bridge).

---

## [0.1.0-beta.21] — 2026-04-24

### Added
- Audit controls for Claude Code sessions and Claude.ai chats (settings
  toggle for capturing queue task prompts and Claude.ai conversation
  metadata).

### Fixed
- macOS cost tracking: per-panel CWD scanning, queue costs surfaced in
  the token panel, file caching to avoid repeated disk reads.
- Memory leaks causing OOM kills in long-running sessions.

---

## [0.1.0-beta.20] — 2026-04-19

### Fixed
- macOS keychain repeated prompts: failure state is now cached so the
  user is not re-prompted after declining keychain access.

---

## [0.1.0-beta.19] — 2026-04-08

### Added
- macOS test suite and CI workflow.

### Fixed
- Sidebar cost scoping now matches exact workspace CWDs instead of
  ancestor directories, eliminating cross-workspace cost bleed.

---

## [0.1.0-beta.18] — 2026-04-07

### Added
- Agent stall detection on Linux (GTK frontend).
- Open-source promotion infrastructure: public-repo sync pipeline,
  desktop entry + icon for the Linux app launcher.

### Fixed
- `install.sh`: correctly handles nested tarball directories.
- Replaced stale URLs and removed personal email from public-facing
  files.

---

## [0.1.0-beta.17] — 2026-04-04

### Added
- Marketing site reworked around pain-point-driven narrative with three
  pillars.
- Agent detection tests; CC → Claude token usage label.
- Linux download enabled; restored Windows WSL install instructions.

### Changed
- Removed the Pro tier — all features are free; Enterprise is reserved
  for auditing and enforced-settings deployments.
- Cleaned up dead code and aligned RPC param structs with the wire
  format (this is the change that established `entry_id` as the
  canonical queue parameter).

### Fixed
- Queue history no longer causes app freezes from unbounded growth and
  repeated I/O.
- Misleading "root access" claim removed from the marketing hero
  headline.
- `thane-cli queue status` / `queue cancel` CLI parameter mismatch
  with the RPC handler (followup fix landed in [Unreleased] for the
  macOS side).

---

## [0.1.0-beta.16] — 2026-04-04

### Changed
- Renamed "Agent Queue" panel; added queue-level sandbox controls.

---

## [0.1.0-beta.15] — 2026-04-04

Release-only — no functional changes since beta.14.

---

## [0.1.0-beta.14] — 2026-03-27

### Fixed
- Sandbox now works correctly for Claude Code on macOS.
- `dist/update-version.sh` fixed for Linux (macOS-specific `sed` syntax
  was failing on GNU `sed`).

### Changed
- Linux and Windows downloads hidden on the marketing site pending
  install-path polish.

---

## [0.1.0-beta.13] — 2026-03-26

### Added
- **Linux parity wave:** audit date-range filter, tab reordering,
  browser improvements, Claude gating, sidebar persistence.

---

## [0.1.0-beta.12] — 2026-03-26

Release-only — packaging fix.

---

## [0.1.0-beta.11] — 2026-03-26

### Added
- `--local` flag on the publish script for testing DMG builds without
  uploading to R2.
- Contact button on the marketing site.
- Cost display settings + token disclaimers.

### Changed
- Marketing build pipeline: Dockerfile passes `NEXT_PUBLIC_*` env vars
  as build args.
- Removed the JSON-RPC API from the marketing-site feature list (it's
  internal plumbing, not a user-facing feature).

---

## [0.1.0-beta.10] — 2026-03-25

### Fixed
- Workspace terminal failing to start; improved workspace-switching
  performance.

---

## [0.1.0-beta.9] — 2026-03-25 and earlier

For releases beta.9 and below, see `git log --oneline` for the change
list. These are early-beta releases prior to public distribution and
predate the structured changelog.

Highlights from this era:
- **beta.7** — Token panel diagnostic logging and improved empty state.
- **beta.6** — Security CLI fallback for keychain credential reads.
- **beta.5** — Claude Code detection includes `~/.local/bin` and
  `nvm`/`npm` paths.
- **beta.4** — Fixed modal-panel-related app freeze; fixed first-launch
  modal-behind-window bug.
- **beta.3** — Initial public beta; `install.sh` Linux bootstrap, full
  unified publish pipeline at `dist/publish.sh`.

---

## Template

When releasing a new version, copy this block, replace the version and
date, and move the relevant entries from `[Unreleased]` here.

```markdown
## [X.Y.Z] — YYYY-MM-DD

### Added
- New features.

### Changed
- Behaviour or default-value changes that existing users will notice.

### Deprecated
- Features still present but slated for removal in a future release.

### Removed
- Features removed in this release.

### Fixed
- Bug fixes.

### Security
- Security-relevant fixes, including the affected severity and any
  exploitation status.
```
