#!/usr/bin/env bash
# Records an automated ghscaff wizard demo (dry-run) and renders
# assets/demo.gif.
#
# The interactive wizard is driven with tmux send-keys — fully scripted.
# Runs with --dry-run: NO repository is created, no writes hit GitHub.
#
# Requirements: tmux, asciinema (v2), agg, ghscaff on PATH, and a
# GITHUB_TOKEN exported in the environment (bypasses the vault passphrase
# so the recording needs no human input). The token itself never appears
# on screen.
#
# Usage: GITHUB_TOKEN=ghp_xxx ./scripts/demo/record.sh

set -euo pipefail

if [ -z "${GITHUB_TOKEN:-}" ]; then
    echo "GITHUB_TOKEN is required (it bypasses the vault passphrase prompt)." >&2
    echo "Usage: GITHUB_TOKEN=ghp_xxx $0" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SESSION="ghscaffdemo"
CAST="$(mktemp --suffix=.cast)"
GIF="$REPO_ROOT/assets/demo.gif"

# Prefer the freshly built binary over any installed release.
if [ -x "$REPO_ROOT/target/release/ghscaff" ]; then
    export PATH="$REPO_ROOT/target/release:$PATH"
fi

mkdir -p "$REPO_ROOT/assets"
tmux kill-session -t "$SESSION" 2>/dev/null || true
tmux new-session -d -s "$SESSION" -x 110 -y 30 \
    -e GITHUB_TOKEN="$GITHUB_TOKEN" bash
tmux set-option -t "$SESSION" status off

COLUMNS=110 LINES=30 asciinema rec --overwrite \
    -c "tmux attach -t $SESSION" "$CAST" &
REC_PID=$!
sleep 1.5

type_text() {
    local text="$1"
    for ((i = 0; i < ${#text}; i++)); do
        tmux send-keys -t "$SESSION" -l "${text:i:1}"
        sleep 0.04
    done
    sleep 0.4
}

line() { type_text "$1"; tmux send-keys -t "$SESSION" Enter; sleep "${2:-1.2}"; }
key()  { tmux send-keys -t "$SESSION" "$1"; sleep "${2:-1}"; }

line "ghscaff --dry-run" 5             # banner + token validation

line "awesome-tool" 1                  # Repository name
line "A demo repository scaffolded by ghscaff" 1   # Description
line "rust,cli,demo" 1                 # Topics
key Enter 1                            # Visibility: Public (first option)
key Enter 1.5                          # Owner: personal account
key Enter 1.5                          # Template: Rust
key Enter 1                            # Default branch: main
key Enter 1                            # Create develop branch? (default)
key Enter 1.5                          # Features (defaults preselected)
key Enter 1.5                          # License: MIT
key Enter 6                            # Apply these changes? -> dry-run plan

sleep 3                                # hold the final frame
tmux kill-session -t "$SESSION" 2>/dev/null || true
wait "$REC_PID" || true

agg --font-size 16 "$CAST" "$GIF"
rm -f "$CAST"

echo "Demo written to $GIF"
