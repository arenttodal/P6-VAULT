#!/usr/bin/env bash
# All automated gates that run on any platform (macOS or Linux).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm typecheck
pnpm test
pnpm build
echo "All checks passed."
