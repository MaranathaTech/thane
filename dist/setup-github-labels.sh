#!/usr/bin/env bash
#
# setup-github-labels.sh — Create standard issue labels on the public thane repo.
#
# This is a one-time setup script. Run it after creating the public repo.
#
# Prerequisites:
#   - gh CLI authenticated with access to MaranathaTech/thane
#
# Usage:
#   ./dist/setup-github-labels.sh
#
set -euo pipefail

REPO="MaranathaTech/thane"

# ── Colors ─────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { echo -e "${BLUE}==>${NC} $*"; }
ok()    { echo -e "${GREEN}==>${NC} $*"; }

create_label() {
    local name="$1"
    local color="$2"
    local description="$3"

    if gh label create "$name" --repo "$REPO" --color "$color" --description "$description" 2>/dev/null; then
        ok "Created: $name"
    else
        info "Already exists: $name"
    fi
}

echo ""
info "Setting up labels on $REPO..."
echo ""

# ── Standard labels ──────────────────────────────────────────────────────
create_label "good first issue"  "7057ff" "Good for newcomers"
create_label "help wanted"       "008672" "Extra attention is needed"
create_label "bug"               "d73a4a" "Something isn't working"
create_label "enhancement"       "a2eeef" "New feature or request"
create_label "documentation"     "0075ca" "Improvements or additions to documentation"
create_label "question"          "d876e3" "Further information is requested"
create_label "wontfix"           "ffffff" "This will not be worked on"
create_label "duplicate"         "cfd3d7" "This issue or pull request already exists"

# ── Platform labels ──────────────────────────────────────────────────────
create_label "linux"             "fca326" "Linux-specific"
create_label "macos"             "bfdadc" "macOS-specific"

# ── Component labels ─────────────────────────────────────────────────────
create_label "core"              "1d76db" "thane-core crate"
create_label "cli"               "5319e7" "thane-cli crate"
create_label "terminal"          "0e8a16" "Terminal emulation (VTE/SwiftTerm)"
create_label "browser"           "fbca04" "Embedded browser (WebKit/WKWebView)"
create_label "sandbox"           "b60205" "Sandboxing (Landlock/seccomp/App Sandbox)"
create_label "audit"             "e99695" "Audit trail and security logging"
create_label "rpc"               "c2e0c6" "JSON-RPC socket API"
create_label "queue"             "d4c5f9" "Agent queue system"
create_label "persistence"       "f9d0c4" "Session save/restore"

echo ""
ok "Label setup complete!"
