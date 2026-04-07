# thane Roadmap

This roadmap outlines the major milestones for thane. It's a living document — priorities may shift based on community feedback.

## Beta (current)

Focus: stability, correctness, and core feature completeness.

- [x] Multi-workspace terminal with split panes
- [x] Embedded browser with Vimium-style keyboard nav
- [x] Per-workspace Landlock + seccomp sandboxing (Linux)
- [x] Per-workspace App Sandbox (macOS)
- [x] Tamper-proof audit trail (SHA-256 hash chain)
- [x] Agent queue with headless execution
- [x] Real-time cost and token tracking
- [x] Sensitive data / PII detection
- [x] Git diff viewer
- [x] Session persistence (auto-save / restore)
- [x] JSON-RPC socket API + CLI
- [x] Ghostty-compatible config format
- [ ] Stabilize sandbox enforcement across kernel versions
- [ ] Improve error messages for sandbox permission denials
- [ ] Automated integration test suite

## 1.0

Focus: production readiness, macOS parity, and contributor experience.

- [ ] macOS feature parity with Linux
- [ ] Plugin / extension system (Lua or WASM)
- [ ] External log shipping (syslog, JSON file, webhook)
- [ ] Per-command sandbox restrictions (in addition to per-workspace)
- [ ] Workspace templates (pre-configured sandbox + config)
- [ ] Tab completion for thane-cli
- [ ] Improved accessibility (screen reader support)
- [ ] Comprehensive documentation site

## Future

Ideas under consideration — no timeline commitment.

- [ ] Windows support (Windows Terminal integration + Windows Sandbox)
- [ ] Remote workspace support (SSH + socket forwarding)
- [ ] Team features (shared audit logs, centralized config)
- [ ] Flatpak distribution
- [ ] Built-in agent marketplace / registry
- [ ] GPU passthrough for ML workloads in sandboxed environments

## How to influence the roadmap

- Open an issue on [GitHub](https://github.com/MaranathaTech/thane/issues) to request a feature or report a bug
- Issues labeled [`help wanted`](https://github.com/MaranathaTech/thane/labels/help%20wanted) are great places to contribute
- See [CONTRIBUTING.md](CONTRIBUTING.md) for how to get started
