# Contributing to thane

Thanks for your interest in contributing to thane! Here's how to get started.

## Building from Source

### Linux (Ubuntu/Debian)

```bash
# Install system dependencies
sudo apt install libgtk-4-dev libvte-2.91-gtk4-dev libwebkitgtk-6.0-dev

# Build
cargo build

# Run tests
cargo test

# Run
cargo run
```

### Linux (Fedora)

```bash
sudo dnf install gtk4-devel vte291-gtk4-devel webkitgtk6.0-devel
cargo build
```

### macOS

```bash
# Build the Rust bridge and CLI
cargo build -p thane-bridge -p thane-cli

# Build the Swift app (requires Xcode 15+, macOS 13+)
cd frontends/macos && swift build
```

## Running Tests

```bash
# Rust (all platforms)
cargo test

# Swift (macOS only)
cd frontends/macos && swift test
```

All tests must pass before submitting a pull request.

## Project Structure

- `crates/` — Rust workspace with 10 crates (all business logic)
- `frontends/macos/` — Swift/AppKit frontend consuming the Rust bridge
- `data/` — Desktop entry, icons, and resources
- `doc/` — Man pages
- `dist/` — Build helpers

All business logic lives in platform-agnostic Rust crates. `thane-gtk` is the Linux frontend; `thane-bridge` + `frontends/macos` is the macOS frontend.

## Submitting Changes

1. Fork the repository
2. Create a feature branch (`git checkout -b my-feature`)
3. Make your changes and add tests
4. Run `cargo test` (and `swift test` on macOS) to verify
5. Commit with a clear message
6. Open a pull request

## Code Style

- Rust: follow `cargo clippy` recommendations
- Swift: follow standard Swift conventions
- Keep platform-specific code out of core crates

## Reporting Issues

Please open an issue on GitHub with:
- What you expected to happen
- What actually happened
- Steps to reproduce
- Your platform and version (`thane --version`)
