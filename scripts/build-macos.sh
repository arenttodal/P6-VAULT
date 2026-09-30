#!/usr/bin/env bash
# Build P6 Vault.app on the owner's Mac (native architecture), after running the checks.
#   ./scripts/build-macos.sh            # checks + release .app + .dmg
#   SKIP_CHECKS=1 ./scripts/build-macos.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== Environment"
sw_vers || true
echo "arch: $(uname -m)"
xcode-select -p >/dev/null 2>&1 || { echo "Install Xcode Command Line Tools: xcode-select --install"; exit 1; }
command -v rustup >/dev/null || { echo "Install Rust: https://rustup.rs"; exit 1; }
command -v node >/dev/null || { echo "Install Node.js 20+ (e.g. brew install node)"; exit 1; }
command -v pnpm >/dev/null || { echo "Install pnpm: npm install -g pnpm (or corepack enable)"; exit 1; }
rustc --version; node --version; pnpm --version

echo "== Dependencies"
pnpm install --frozen-lockfile

if [[ "${SKIP_CHECKS:-0}" != "1" ]]; then
  echo "== Checks"
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  pnpm typecheck
  pnpm test
fi

echo "== Build (release, $(uname -m))"
pnpm tauri build

APP="target/release/bundle/macos/P6 Vault.app"
echo
echo "Built: $APP"
ls -1 target/release/bundle/dmg/*.dmg 2>/dev/null || true
echo "The app is not signed/notarized with a Developer ID. A locally built app opens normally"
echo "from Finder on this Mac. Copy it to /Applications to install."
