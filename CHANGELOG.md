# Changelog

All notable changes to thane are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

When adding an entry for the next release, copy the **Template** at the
bottom of this file into a new section under `[Unreleased]`.

---

## [Unreleased]

### Added

- Foundational enterprise documentation:
  [`COMPLIANCE.md`](dist/public/COMPLIANCE.md) (control mapping for
  SOC 2, ISO 27001, HIPAA, PCI-DSS v4, NIST 800-53 Rev. 5),
  [`API.md`](dist/public/API.md) (full JSON-RPC method reference for the
  51 methods exposed over the socket), and additional sections in
  [`ENTERPRISE.md`](dist/public/ENTERPRISE.md) covering Kandji + Intune
  MDM profiles, daemon-at-login operational guide, decommissioning
  playbook, and a longer-form Loki + Grafana setup recipe.
- `CHANGELOG.md` at the repo root following the Keep a Changelog format.

### Changed

- `~/.claude/CLAUDE.md` global instructions template bumped to v5:
  dependency chaining is now opt-in (default is independent phase
  submission), reversing the previous v4 default that auto-chained
  every queued phase.

### Fixed

- macOS: `thane-cli queue cancel` and `queue status` now work end-to-end.
  Previously the macOS RPC handler expected `id` while the CLI sent
  `entry_id`, producing `Missing 'id' parameter`. Both names are now
  accepted; `entry_id` is canonical and `id` is documented as a
  deprecated fallback.
- macOS: queue executor now terminates the running subprocess when an
  entry is cancelled. Previously the cancel only flipped the status
  flag, leaving the `claude` subprocess running until it finished on
  its own.
- Linux: same kill-on-cancel fix in the GTK frontend's queue poll loop.
- macOS: `claude` binary now resolved via comprehensive path search
  (`~/.local/bin`, NVM versions, Homebrew paths, `~/.cargo/bin`, plus a
  `zsh -lc 'command -v claude'` fallback). Fixes Exit 127 failures
  when launchd's minimal PATH did not include the user's tool install
  location.
- macOS: spawned queue subprocesses now receive an augmented PATH
  covering Homebrew (Intel + Apple Silicon), `/usr/local`,
  `~/.local/bin`, `~/.cargo/bin`, NVM, etc.
- macOS: bridge now injects `~/.claude/CLAUDE.md` instructions at
  startup. Previously only the GTK frontend did this, so macOS users
  got stale or missing instructions.

### Removed

- Dead Swift duplicate of the CLAUDE.md injector in `AppDelegate.swift`
  (was using a stale v3 marker).

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
