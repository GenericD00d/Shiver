#!/usr/bin/env bash
# Shiver's local checks, the same ones CI runs. Run before pushing:
#
#   scripts/check.sh           everything (needs Rust, Bun, Node and the network)
#   scripts/check.sh --quick   docs, versions and the unit tests that need no build
#
# CI also builds the Android app, which needs the Android SDK and NDK; see .github/workflows/checks.yml.
set -euo pipefail
cd "$(dirname "$0")/.."

quick=false
[ "${1:-}" = "--quick" ] && quick=true

step() { printf '\n== %s\n' "$*"; }

step "Docs follow the standard"
python3 scripts/check-docs.py

step "Versions agree"
python3 scripts/check-version.py

step "Web and bridge unit tests"
bun test shared/web desktop/bridge mobile/bridge

step "Companion plugin"
(cd plugin && for file in server/*.js client/*.js; do node --check "$file"; done && node --test test/*.test.js)

if $quick; then
  printf '\nQuick checks passed.\n'
  exit 0
fi

step "Sharkord still has what the bridges match"
python3 scripts/check-sharkord.py

# the bridges are include_str!'d, so they are built before the Rust crates compile
step "Desktop: install, bridge, types, audit"
(cd desktop && bun install --frozen-lockfile && bun run build:bridge && bunx tsc --noEmit && bun audit)

step "Mobile: install, bridge, rail, types, audit"
(cd mobile && bun install --frozen-lockfile && bun run build:bridge && bun run build:rail && bunx tsc --noEmit && bun audit)

step "Rust: format, lints, tests"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

step "Rust: dependency audit"
if cargo audit --version >/dev/null 2>&1; then
  cargo audit
else
  echo "cargo-audit isn't installed (cargo install cargo-audit --locked); CI runs it"
fi

printf '\nAll checks passed.\n'
