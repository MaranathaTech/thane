# thane

**The AI-native terminal workspace manager.**

thane combines a terminal emulator, embedded browser, and rich metadata sidebar into project-centric workspaces — designed from the ground up for AI coding agents.

[Website](https://getthane.com) | [Documentation](https://getthane.com/docs) | [Features](https://getthane.com/features)

## Why thane?

AI coding agents need more than a plain terminal. thane gives them — and you — a structured workspace with:

- **Multi-workspace terminal** with split panes and vim-style navigation
- **Embedded browser** with JavaScript scripting and Vimium-style keyboard nav
- **Agent monitoring** — activity detection, cost tracking, token usage per workspace
- **Agent queue** — submit tasks for headless execution, monitor progress
- **Sandbox mode** — Landlock + seccomp isolation per workspace
- **JSON-RPC socket API** — full programmatic control for AI agents
- **Session persistence** — pick up where you left off across restarts
- **Notifications** with desktop integration and per-workspace history
- **Audit log** — track security events with severity filtering; ship to syslog, S3, Splunk HEC, Datadog Logs, Grafana Loki, or any webhook (see [AUDIT_LOG.md](AUDIT_LOG.md) and [COMPLIANCE.md](COMPLIANCE.md))
- **Git diff viewer** — inline diffs per pane, no context switching
- **Ghostty-compatible config** format
- **Leader key mode** (tmux-style `Ctrl+B` prefix)

## Platforms

| Platform | Stack | Status |
|----------|-------|--------|
| Linux | GTK4 + VTE + WebKitGTK | Available |
| macOS | AppKit + SwiftTerm + WKWebView | Available |

## Install

### Linux

```bash
curl -fsSL https://getthane.com/install.sh | bash
```

Or build from source:

```bash
# Install system dependencies (Ubuntu/Debian)
sudo apt install libgtk-4-dev libvte-2.91-gtk4-dev libwebkitgtk-6.0-dev

# Fedora
sudo dnf install gtk4-devel vte291-gtk4-devel webkitgtk6.0-devel

# Build
cargo build --release

# Install
sudo install -Dm755 target/release/thane /usr/local/bin/thane
sudo install -Dm755 target/release/thane-cli /usr/local/bin/thane-cli
sudo install -Dm755 target/release/thane-daemon /usr/local/bin/thane-daemon
```

### macOS

Download the latest `.dmg` from [getthane.com](https://getthane.com).

Or build from source:

```bash
# Build the Rust bridge and CLI
cargo build --release -p thane-bridge -p thane-cli

# Build the Swift app (requires Xcode 15+, macOS 13+)
cd frontends/macos && swift build -c release
```

## Usage

```bash
# Launch thane
thane

# Control a running instance via CLI
thane-cli ping
thane-cli workspace list
thane-cli workspace create --title "My Project" --cwd ~/projects/myapp
thane-cli surface split-right
thane-cli browser open https://localhost:3000
```

### Background daemon — queue without the GUI

The `thane-daemon` process owns the IPC socket and runs the agent queue. Once
installed it starts at every login, so `thane-cli queue submit ...` works
immediately and queued tasks execute whether or not the GUI is open.

```bash
# macOS — installs ~/Library/LaunchAgents/com.thane.daemon.plist
thane-daemon install-launch-agent

# Linux — installs ~/.config/systemd/user/thane-daemon.service
thane-daemon install-user-service

# Health check / lifecycle
thane-cli status              # daemon_running, daemon_pid, socket_path, uptime_secs
thane-cli daemon start        # spawn in the background if not already running
thane-cli daemon restart      # SIGTERM existing, then spawn fresh
```

The GUI app auto-installs the LaunchAgent on first launch (set
`daemon-start-at-login = false` in your config to opt out) and steps aside
when an external daemon is already serving the socket.

## Configuration

thane reads Ghostty-format config files:

1. `~/.config/ghostty/config` (Ghostty compatibility)
2. `~/.config/thane/config` (overrides)

```
# ~/.config/thane/config
font-family = JetBrains Mono
font-size = 14
scrollback-limit = 50000
```

## Socket API

thane exposes a JSON-RPC 2.0 API over a Unix domain socket. AI agents running inside thane can use it to create workspaces, send notifications, split panes, and more.

Environment variables available in spawned shells:

- `THANE_WORKSPACE_ID` — current workspace UUID
- `THANE_SURFACE_ID` — current pane UUID
- `THANE_SOCKET_PATH` — path to the socket

```bash
# Example: send a notification from an agent
echo '{"jsonrpc":"2.0","method":"notification.send","params":{"title":"Build","body":"Tests passed"},"id":1}' \
  | socat - UNIX-CONNECT:$THANE_SOCKET_PATH
```

## Architecture

All business logic lives in platform-agnostic Rust crates. Platform-specific code is isolated to `thane-gtk` (Linux) and `thane-bridge` + `frontends/macos` (macOS).

```
crates/
├── thane-core       # Workspaces, sandbox, audit, agent queue, config, git
├── thane-rpc        # JSON-RPC method dispatch
├── thane-ipc        # Unix socket server
├── thane-persist    # Session save/restore, audit store, queue history
├── thane-platform   # Platform dirs, notifications, Landlock, seccomp
├── thane-terminal   # Terminal surface trait + VTE backend
├── thane-browser    # Browser surface trait + WebKit backend
├── thane-gtk        # Linux frontend (GTK4 + VTE + WebKitGTK)
├── thane-cli        # CLI client
├── thane-daemon     # Headless background daemon (IPC + queue executor)
└── thane-bridge     # macOS FFI bridge (UniFFI → Swift)

frontends/
└── macos/           # Swift/AppKit frontend
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+T` | New workspace |
| `Ctrl+Shift+D` | Split right |
| `Ctrl+Shift+E` | Split down |
| `Alt+H/J/K/L` | Focus pane (vim-style) |
| `Ctrl+Shift+Z` | Toggle pane zoom |
| `Ctrl+B` | Enter leader mode |
| `Ctrl+,` | Settings |
| `Ctrl+Shift+F` | Find in terminal |
| `Ctrl+Shift+G` | Git diff |
| `Ctrl+Shift+B` | Toggle sidebar |
| `F1` | Help (full shortcut reference) |

See the in-app Help panel (`F1`) for the complete list.

## Enterprise

For teams with SOC 2 / ISO 27001 / HIPAA / PCI / NIST 800-53 audit requirements,
thane ships a full audit pipeline out of the box:

- **27 event types** covering shell, file, network, browser, sandbox, queue,
  and agent activity
- **Per-event HMAC-SHA256 + tamper-evident hash chain** — events cannot be
  silently edited
- **AES-256-GCM encryption at rest** for rotated segments
- **Configurable redaction** of secrets, tokens, and PII before signing
- **External delivery** to syslog, S3 (with Object Lock), Splunk HEC, Datadog
  Logs, or any HTTPS webhook
- **Offline verification** for auditors with `thane-cli audit verify`

For details:
- [AUDIT_LOG.md](AUDIT_LOG.md) — event schema, redaction modes, per-sink
  configuration, operator runbook
- [COMPLIANCE.md](COMPLIANCE.md) — control mapping for SOC 2, ISO 27001,
  HIPAA, PCI-DSS v4, NIST 800-53 Rev. 5
- [ENTERPRISE.md](ENTERPRISE.md) — admin / MDM deployment guide,
  enterprise policy file, daemon-at-login operational guide,
  decommissioning playbook
- [API.md](API.md) — JSON-RPC method reference (51 methods)
- [CHANGELOG.md](../../CHANGELOG.md) — release history

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### Building from source

```bash
# System dependencies (Ubuntu/Debian)
sudo apt install libgtk-4-dev libvte-2.91-gtk4-dev libwebkitgtk-6.0-dev

# Build
cargo build

# Run tests
cargo test

# Run
cargo run
```

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE) for the full text.

Commercial licensing is available — contact us at [getthane.com](https://getthane.com).
