#!/usr/bin/env bash
#
# The PR gate, runnable in one command. This is the SINGLE SOURCE OF TRUTH for
# "did my change pass CI's checks" — `.github/workflows/pr-checks.yml`'s verify
# job runs this exact script, and so should you (and any agent) before pushing.
#
# It exists because "cargo test passes" is not the same as "CI is green": CI
# also enforces `clippy -D warnings` (on the pinned toolchain, see
# rust-toolchain.toml), `fmt --check`, `tsc --noEmit`, the Vitest frontend
# suite, and commitlint on the commit range. Running the real gates locally is
# the only reliable way to avoid the "passed here, failed in CI" round trip.
#
# Usage:
#   scripts/checks.sh                 # run every gate, including commitlint
#   CHECKS_SKIP_COMMITLINT=1 ...      # skip commitlint (CI runs it separately)
#   CHECKS_BASE=origin/master ...     # commit range base for commitlint
#
# Fails fast on the first failing gate with a nonzero exit.
set -euo pipefail

# Always operate from the repo root regardless of where we're invoked.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

step() { printf '\n\033[1;34m==>\033[0m %s\n' "$1"; }

step "TypeScript type-check (tsc --noEmit)"
npx tsc --noEmit

step "Frontend tests (vitest run)"
npx vitest run

step "Rust format check (cargo fmt --all -- --check)"
( cd src-tauri && cargo fmt --all -- --check )

step "Rust clippy (--all-targets -D warnings)"
( cd src-tauri && cargo clippy --all-targets -- -D warnings )

step "Rust tests (cargo test)"
( cd src-tauri && cargo test )

# Core + runner live in the workspace but `cargo test` from src-tauri only runs
# the demeteo package (cargo scopes to the cwd's package). Test them explicitly
# so a change to either crate is actually exercised locally.
step "Core + runner tests"
cargo test -p demeteo-core -p demeteo-runner

if [ "${CHECKS_SKIP_COMMITLINT:-0}" = "1" ]; then
  step "Commitlint — skipped (CHECKS_SKIP_COMMITLINT=1)"
else
  BASE="${CHECKS_BASE:-origin/master}"
  if git rev-parse --verify --quiet "$BASE" >/dev/null; then
    step "Commitlint ($BASE..HEAD)"
    npx --no-install commitlint --from "$BASE" --to HEAD --config .commitlintrc.json
  else
    step "Commitlint — skipped ($BASE not found; run 'git fetch origin master')"
  fi
fi

printf '\n\033[1;32m✔ all checks passed\033[0m\n'
