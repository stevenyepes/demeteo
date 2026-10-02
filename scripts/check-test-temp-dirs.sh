#!/usr/bin/env bash
#
# Tests take their scratch space from `demeteo_core::test_dir` (in-crate:
# `crate::support::test_dir`), never from `std::env::temp_dir()` directly.
# Covered: core's tests, src-tauri's tests, and the runner's `*_tests.rs`;
# the other two reach the helper through core's `test-support` feature.
#
# A bare `temp_dir().join(format!("demeteo-…-{nanos}"))` is never removed:
# most fixtures hand the path to `build_core_context` and drop it. Each
# `cargo test` leaked about forty trees that way, until a tmpfs `/tmp` held
# 15 GB of them and the worktree tests' 20 GiB disk guard started failing on
# a perfectly healthy tree. `test_dir.rs` carries the mechanism; this gate
# keeps the next fixture from copying the old line out of habit.
#
# A use that genuinely needs the system temp dir itself — asserting where a
# path resolves, say — opts out on the same line or the line above with
#     // temp-dir-ok: <why>
# and the reason is what a reviewer reads.

set -euo pipefail
cd "$(dirname "$0")/.."

hits=$( {
  grep -rn --include='*.rs' 'temp_dir()' crates/demeteo-core/tests src-tauri/tests
  grep -rn --include='*_tests.rs' 'temp_dir()' crates/demeteo-runner/src
} | grep -v '^crates/demeteo-core/tests/support/test_dir.rs:' || true)

bad=()
while IFS= read -r hit; do
  [ -z "$hit" ] && continue
  file=${hit%%:*}
  rest=${hit#*:}
  line=${rest%%:*}
  if grep -q 'temp-dir-ok:' <<<"$rest"; then continue; fi
  if [ "$line" -gt 1 ] && sed -n "$((line - 1))p" "$file" | grep -q 'temp-dir-ok:'; then continue; fi
  bad+=("$hit")
done <<<"$hits"

if [ ${#bad[@]} -gt 0 ]; then
  printf 'Raw temp_dir() in tests — use demeteo_core::test_dir::{TestDir, scratch}\n' >&2
  printf '(or mark a deliberate use with `// temp-dir-ok: <why>`):\n\n' >&2
  printf '  %s\n' "${bad[@]}" >&2
  exit 1
fi
echo "tests take scratch space from demeteo_core::test_dir"
