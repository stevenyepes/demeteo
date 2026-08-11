#!/usr/bin/env bash
#
# The PR gate, runnable in one command. This is the SINGLE SOURCE OF TRUTH for
# "did my change pass CI's checks" — `.github/workflows/pr-checks.yml`'s verify
# job runs this exact script, and so should you (and any agent) before pushing.
#
# It exists because "cargo test passes" is not the same as "CI is green": CI
# also enforces `clippy -D warnings` (on the pinned toolchain, see
# rust-toolchain.toml), `fmt --check`, `tsc --noEmit`, Biome, the Vitest
# frontend suite, and commitlint on the commit range. Running the real gates
# locally is the only reliable way to avoid the "passed here, failed in CI"
# round trip.
#
# Usage:
#   scripts/checks.sh                 # run every gate, including commitlint
#   npm run checks:code               # every gate EXCEPT commitlint — see below
#   CHECKS_SKIP_COMMITLINT=1 ...      # skip commitlint (CI runs it separately)
#   CHECKS_BASE=origin/master ...     # commit range base for commitlint
#
# Why `checks:code` exists, and who should use it. Commitlint here judges the
# *range* `origin/master..HEAD`, which is the right gate for a branch a human is
# about to push and the wrong one for a branch mid-run inside Demeteo. There,
# every commit in that range is orchestrator plumbing — one per ticket, plus the
# subtask merges — and the finalize step squashes the lot into a single commit
# whose message it validates against this repo's real `commit-msg` hook before
# publishing. So a mid-run commitlint gate judges commits that are already
# scheduled for deletion, and it fails on them: an agent cannot fix a message it
# did not write and will not survive the squash, so the verdict feeds a rework
# cycle that closes nothing. Demeteo's own default test command should therefore
# be `npm run checks:code`; the `pre-push` hook and CI keep running the full
# `checks`, which is where linting the range is meaningful.
#
# Fails fast on the first failing gate with a nonzero exit.
set -euo pipefail

# Always operate from the repo root regardless of where we're invoked.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

step() { printf '\n\033[1;34m==>\033[0m %s\n' "$1"; }

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
RUNNER_SUPPORTED=1
if [[ "$HOST_TRIPLE" == *-windows-* ]]; then
  RUNNER_SUPPORTED=0
fi

step "TypeScript type-check (tsc --noEmit)"
npx tsc --noEmit

# The frontend counterpart to `check-doc-refs.sh`: the AGENTS.md §3 rules a
# compiler cannot see. Chief among them, `noRestrictedImports` denies
# `@tauri-apps/api/core` under components/, hooks/ and context/, so "never
# invoke() raw in a component" is enforced rather than reviewed. biome.jsonc
# keeps everything else advisory — only an error fails this gate.
step "Frontend lint (biome check)"
npx --no-install biome check .

# The one AGENTS.md §4 rule nothing above can see: a class name is just a string
# to tsc, to Biome, and to jsdom, and Tailwind emits no rule and no warning for a
# candidate it cannot resolve. That is how `font-outfit` stayed on headings
# across 32 files while they all rendered in Inter. This compiles the real
# stylesheet and fails on a class used but never defined; what it deliberately
# does not cover is recorded in its own header.
step "Frontend class names (used vs defined)"
node scripts/check-classes.mjs

step "Frontend tests (vitest run)"
npx vitest run

step "Rust format check (cargo fmt --all -- --check)"
( cd src-tauri && cargo fmt --all -- --check )

step "Rust clippy (--all-targets -D warnings)"
( cd src-tauri && cargo clippy --all-targets -- -D warnings )

# Comment rot, in the two forms a machine can see. `cargo doc` resolves every
# `[`Foo`]` intra-doc link (denied via `[workspace.lints.rustdoc]`), catching a
# rename the prose was not revisited for; check-doc-refs.sh resolves the file
# paths comments cite and rejects a paragraph copied into more than one file.
# Neither is reachable from clippy: it does not evaluate rustdoc lints, and no
# lint reads a `//` comment at all.
step "Rust doc links (cargo doc --no-deps)"
if [[ "$RUNNER_SUPPORTED" == 1 ]]; then
  cargo doc --no-deps -p demeteo-core -p demeteo-runner
else
  cargo doc --no-deps -p demeteo-core
  step "Runner docs — skipped (Linux-only binary; host: $HOST_TRIPLE)"
fi

step "Doc references + duplicate comment blocks"
scripts/check-doc-refs.sh

step "Rust tests (cargo test)"
( cd src-tauri && cargo test )

# Core + runner live in the workspace but `cargo test` from src-tauri only runs
# the demeteo package (cargo scopes to the cwd's package). Test them explicitly
# so a change to either crate is actually exercised locally.
step "Core + runner tests"
if [[ "$RUNNER_SUPPORTED" == 1 ]]; then
  cargo test -p demeteo-core -p demeteo-runner
else
  cargo test -p demeteo-core
  step "Runner tests — skipped (Linux-only binary; host: $HOST_TRIPLE)"
fi

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
