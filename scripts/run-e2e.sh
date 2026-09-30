#!/usr/bin/env bash
# Run the WebDriver end-to-end test on Linux under Xvfb with an isolated data dir.
set -euo pipefail
APP=${1:-target/debug/p6-vault}
SHOTS=${2:-e2e-shots}
HOME_DIR=${E2E_HOME:-$(mktemp -d)}
export XDG_DATA_HOME="$HOME_DIR/data" XDG_CONFIG_HOME="$HOME_DIR/config" XDG_CACHE_HOME="$HOME_DIR/cache"
Xvfb :99 -screen 0 1440x900x24 >/dev/null 2>&1 & XVFB=$!
export DISPLAY=:99
sleep 1
tauri-driver --port 4444 >"$HOME_DIR/tauri-driver.log" 2>&1 & TD=$!
trap 'kill $TD $XVFB 2>/dev/null || true' EXIT
sleep 2
node scripts/e2e.mjs "$APP" "$SHOTS"
