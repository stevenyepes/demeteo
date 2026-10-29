#!/usr/bin/env bash
#
# Decides which pr-checks.yml jobs a pull request needs, from the files it
# changed. It answers three questions, each a line on stdout that the workflow
# appends to $GITHUB_OUTPUT:
#
#   frontend=   run scripts/checks.sh frontend (which also builds hub-web)
#   rust=       run scripts/checks.sh rust, the conformance suites, cross-os clippy
#   os_tests=   also run the core tests on macOS and Windows
#   hub=        run the Hub job: Docker Postgres + OpenBao and a release image
#               build. Every change `rust` runs for, and deploy/hub/ besides;
#               the image copies `crates/` and `src-tauri/`, and nothing in
#               `hub-web/` or the docs
#
# ## Unknown means yes
#
# Every path is classified by an allowlist of what is known NOT to matter to a
# job, and anything the list does not name runs everything. The opposite shape —
# a list of paths that trigger a job — goes stale silently: a new top-level
# config file matches no trigger, the job is skipped, and the skip reads as a
# pass. A path added to the tree tomorrow costs CI minutes here until someone
# classifies it, which is the cheap direction to be wrong in.
#
# ## The trees are not as separate as their directories suggest
#
# Each exception below is a file one toolchain reads out of the other's tree,
# found by grepping for it rather than assumed:
#
#   src/components/canvas/__fixtures__/   core's `canvas_fixtures_are_current`
#   docs-site/workflow-schema-v2.json     core's `published_schema_is_current`
#   src-tauri/workflows/                  imported by vitest suites in src/lib/
#
# And a removed or renamed file anywhere runs the Rust job, because
# check-doc-refs.sh resolves the paths Rust comments cite against every tracked
# file: deleting a doc can break a comment in a crate the PR never opened.
#
# ## When the macOS and Windows test legs earn their minutes
#
# Compile coverage (`cross-os`, clippy --all-targets) runs on every Rust change;
# that is what catches a `cfg`-gated item that does not build. Running the
# *tests* there is a separate question, because on Windows they take five times
# what they take on Linux. Every Windows-only test failure in this repo's
# history came from a PR that changed code talking to the OS — `adapters/`,
# `ports/`, the native-Windows execution work — and none from one confined to
# `domain/` or `application/`. So the tests run unless the whole Rust diff is:
#
#   - core's `domain/` or `application/` (and their tests), where the changed
#     file itself neither sits behind a platform `cfg` nor touches the
#     filesystem, a process, the environment or a socket — see OS_FACING; or
#   - `src-tauri/` or `crates/demeteo-runner/`, whose tests no cross-os leg has
#     ever run (they select `-p demeteo-core`), so skipping loses nothing.
#
# A master push always runs everything, so a miss here surfaces on the merge
# commit rather than never.
#
# Usage:
#   scripts/ci-changes.sh < changed.tsv   # status<TAB>path<TAB>previous_path
#   scripts/ci-changes.sh --all           # every job (pushes, oversized PRs)
#   scripts/ci-changes.sh --self-test
#
# CI_CHANGES_ROOT overrides the directory changed files are read from.
set -euo pipefail

ROOT="${CI_CHANGES_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

OS_FACING='cfg!?\(.*(windows|unix|target_os|target_family)|std::fs|tokio::fs|std::process|Command::new|std::env::|TcpListener|UnixStream|tempfile|temp_dir|test_dir|TestDir'

# Sets KIND for one path. First match wins, so the cross-tree exceptions sit
# above the directory they are carved out of.
classify() {
  case "$1" in
    src/components/canvas/__fixtures__/*|src-tauri/workflows/*) KIND=frontend+rust ;;
    docs-site/workflow-schema-v2.json) KIND=rust-portable ;;

    crates/demeteo-core/src/domain/*|crates/demeteo-core/src/application/*) KIND=rust-pure ;;
    crates/demeteo-core/tests/domain/*|crates/demeteo-core/tests/application/*) KIND=rust-pure ;;
    src-tauri/Cargo.toml|crates/*/Cargo.toml) KIND=rust ;;
    src-tauri/*|crates/demeteo-runner/*) KIND=rust-portable ;;
    crates/*|Cargo.toml|Cargo.lock|rust-toolchain.toml|.cargo/*|.config/nextest.toml) KIND=rust ;;
    scripts/check-doc-refs.sh|scripts/check-test-temp-dirs.sh) KIND=rust-portable ;;

    deploy/hub/*) KIND=hub ;;

    src/*|public/*|index.html|vite.config.ts|vitest.config.ts) KIND=frontend ;;
    hub-web/*|vite.hub-web.config.ts) KIND=frontend ;;
    tsconfig.json|tsconfig.node.json|biome.jsonc|scripts/check-classes.mjs) KIND=frontend ;;

    docs/*|docs-site/*|mkdocs.yml|LICENSE|.design-sync/*) KIND=none ;;
    README.md|CHANGELOG.md|CONTRIBUTING.md|CODE_OF_CONDUCT.md) KIND=none ;;
    AGENTS.md|CLAUDE.md|AGENT_INTEGRATION.md) KIND=none ;;
    .github/workflows/docs.yml|.github/workflows/build.yml) KIND=none ;;
    .github/workflows/promote.yml|.github/workflows/lint-commits.yml) KIND=none ;;

    *) KIND=all ;;
  esac
}

touches_os() { [ -f "$ROOT/$1" ] && grep -Eq "$OS_FACING" "$ROOT/$1"; }

# Reads the TSV on stdin, sets FRONTEND / RUST / OS_TESTS / HUB, and explains each
# flag's first cause on stderr.
decide() {
  FRONTEND=false RUST=false OS_TESTS=false HUB=false
  local status path prev p
  while IFS=$'\t' read -r status path prev || [ -n "${status:-}" ]; do
    case "$status" in
      removed|renamed)
        [ "$RUST" = true ] || echo "rust: $path was $status (doc-refs resolves against every tracked file)" >&2
        RUST=true ;;
    esac
    for p in "$path" "${prev:-}"; do
      [ -n "$p" ] || continue
      classify "$p"
      case "$KIND" in
        all|frontend|frontend+rust)
          [ "$FRONTEND" = true ] || echo "frontend: $p ($KIND)" >&2
          FRONTEND=true ;;
      esac
      case "$KIND" in
        all|rust|rust-pure|rust-portable|frontend+rust)
          [ "$RUST" = true ] || echo "rust: $p ($KIND)" >&2
          RUST=true ;;
      esac
      case "$KIND" in
        all|rust|rust-pure|rust-portable|frontend+rust|hub)
          [ "$HUB" = true ] || echo "hub: $p ($KIND)" >&2
          HUB=true ;;
      esac
      case "$KIND" in
        all|rust)
          [ "$OS_TESTS" = true ] || echo "os_tests: $p ($KIND)" >&2
          OS_TESTS=true ;;
        rust-pure)
          if touches_os "$p"; then
            [ "$OS_TESTS" = true ] || echo "os_tests: $p (talks to the OS or is cfg-gated)" >&2
            OS_TESTS=true
          fi ;;
      esac
    done
  done
}

emit() { printf 'frontend=%s\nrust=%s\nos_tests=%s\nhub=%s\n' "$FRONTEND" "$RUST" "$OS_TESTS" "$HUB"; }

self_test() {
  local tmp failed=0
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/crates/demeteo-core/src/domain" "$tmp/crates/demeteo-core/tests/application"
  echo 'pub fn total(a: u32, b: u32) -> u32 { a + b }' >"$tmp/crates/demeteo-core/src/domain/pure.rs"
  echo '#[cfg(windows)] fn sep() {}' >"$tmp/crates/demeteo-core/src/domain/gated.rs"
  echo 'let dir = TestDir::new();' >"$tmp/crates/demeteo-core/tests/application/disk.rs"
  ROOT="$tmp"

  # expected "frontend rust os_tests hub" | TSV rows, `;`-separated, `,` for TAB
  local cases=(
    "false false false false|modified,docs/ARCHITECTURE.md"
    "false false false false|modified,AGENTS.md;modified,.github/workflows/build.yml"
    "false true false false|removed,docs/OLD.md"
    "false true false false|renamed,docs/NEW.md,docs/OLD.md"
    "true false false false|modified,src/components/TopBar.tsx"
    "true false false false|modified,hub-web/src/views/Run.tsx"
    "true false false false|modified,vite.hub-web.config.ts"
    "false false false true|modified,deploy/hub/Caddyfile"
    "true true false true|modified,src/components/canvas/__fixtures__/a.v2.json"
    "true true false true|modified,src-tauri/workflows/code-review.json"
    "false true false true|modified,docs-site/workflow-schema-v2.json"
    "false true false true|modified,src-tauri/src/tray.rs"
    "false true false true|modified,crates/demeteo-runner/src/main.rs"
    "false true false true|modified,crates/demeteo-core/src/domain/pure.rs"
    "false true true true|modified,crates/demeteo-core/src/domain/gated.rs"
    "false true true true|modified,crates/demeteo-core/tests/application/disk.rs"
    "false true true true|modified,crates/demeteo-core/src/adapters/merge.rs"
    "false true true true|modified,crates/demeteo-core/src/ports/notification.rs"
    "false true true true|modified,crates/demeteo-core/tests/fixtures/x.jsonl"
    "false true true true|modified,Cargo.lock"
    "false true true true|modified,.config/nextest.toml"
    "false true true true|modified,src-tauri/Cargo.toml"
    "false true true true|renamed,crates/demeteo-core/src/domain/pure.rs,crates/demeteo-core/src/shared/pure.rs"
    "true true true true|modified,package.json"
    "true true true true|modified,.github/workflows/pr-checks.yml"
    "true true true true|modified,scripts/checks.sh"
    "true true true true|added,some-new-top-level.toml"
    "true true false true|modified,src/App.tsx;modified,crates/demeteo-core/src/domain/pure.rs"
  )
  local c want rows got
  for c in "${cases[@]}"; do
    want="${c%%|*}"
    rows="$(printf '%s' "${c#*|}" | tr ';,' '\n\t')"
    decide <<<"$rows" 2>/dev/null
    got="$FRONTEND $RUST $OS_TESTS $HUB"
    if [ "$got" != "$want" ]; then
      printf 'FAIL  want [%s] got [%s]  %s\n' "$want" "$got" "${c#*|}" >&2
      failed=1
    fi
  done
  rm -rf "$tmp"
  [ "$failed" = 0 ] || exit 1
  printf 'ci-changes self-test: %d cases ok\n' "${#cases[@]}"
}

case "${1:-}" in
  --self-test) self_test ;;
  --all) FRONTEND=true RUST=true OS_TESTS=true HUB=true; emit ;;
  "") decide; emit ;;
  *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
esac
