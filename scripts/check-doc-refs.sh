#!/usr/bin/env bash
#
# Two gates against comment rot, both mechanical.
#
# ## 1. Path references must resolve
#
# Comments here cite source paths constantly — `adapters/worktree/git_ops.rs`,
# `domain/intercept.rs` — because the rationale for a module usually lives next
# to a pointer at the code it was extracted from. Those pointers rot in exactly
# one way, and it is the refactor this codebase does most: a module outgrows
# ~400 LOC, `foo.rs` becomes `foo/`, and every comment naming `foo.rs` is now
# describing a file that does not exist. Nothing catches it. `cargo build` never
# reads a comment, and a reviewer reading the diff for `foo/` has no reason to
# grep the rest of the tree for its old name.
#
# So: extract every backtick-quoted path from every Rust comment and require it
# to name something real.
#
# The check is deliberately narrow, because a noisy gate gets disabled. An
# unresolved ref is reported only when something of that name plausibly exists
# elsewhere in the tree — `foo.rs` unresolved but `foo/` present, or the same
# basename sitting at a different path. That is precisely the "it moved"
# signature, and it is what makes the gate self-limiting: comments naming things
# that were never repo files at all — `artifacts/task-list.json`, `report.md`,
# `.claude/`, a path inside a Workflow's simulated project — produce no such
# evidence and are passed over. The alternative, a hand-maintained allowlist,
# would rot exactly the way the comments do.
#
# ## 2. No duplicated comment blocks
#
# The same paragraph pasted into five modules is five things to update when the
# rule changes, and in practice four of them do not get updated. A rule that
# applies repo-wide belongs in AGENTS.md, cited by name, not copied. Blocks of
# >=3 identical comment lines appearing in more than one file fail.
#
# Both gates print the fix, not just the failure.
#
# Usage:
#   scripts/check-doc-refs.sh          # both gates
#   DOC_REFS_DUPES=0 ...               # path gate only
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail=0

# Every tracked file, plus every tracked directory, as resolution targets.
# Directories matter: a comment that says `steps/sequence/` is correct and must
# not be reported just because no file has that exact name.
manifest="$(mktemp)"
trap 'rm -f "$manifest" "$manifest.dirs" "$manifest.refs" "$manifest.blocks"' EXIT
# `--others --exclude-standard` includes new, not-yet-committed files: a file
# added in this change must be checked before it is committed, not after.
git ls-files --cached --others --exclude-standard >"$manifest"
sed 's#/[^/]*$##' "$manifest" | sort -u | sed 's#$#/#' >"$manifest.dirs"

# Rust sources we own. Vendored code documents its own upstream layout and is
# not ours to correct.
sources() {
  git ls-files --cached --others --exclude-standard '*.rs' | grep -v '/vendor/'
}

# ── Gate 1: path references resolve ──────────────────────────────────────────

: >"$manifest.refs"
while IFS= read -r file; do
  # Comment lines only (`//`, `///`, `//!`), then backtick-quoted paths.
  # A file with no comments, or comments with no refs, is the common case — not
  # an error — so neither grep's "no match" exit may trip `set -e`.
  { grep -hE '^[[:space:]]*//' "$file" 2>/dev/null || true; } \
    | { grep -oE '`[A-Za-z0-9_./-]+\.(rs|md|toml|sh|yml|yaml|json)`|`[A-Za-z0-9_./-]+/`' || true; } \
    | tr -d '`' \
    | while IFS= read -r ref; do
        printf '%s\t%s\n' "$file" "$ref"
      done
done < <(sources) >>"$manifest.refs"

while IFS=$'\t' read -r file ref; do
  [ -n "$ref" ] || continue

  # Resolve: exact tracked path, or any tracked path ending in `/<ref>`
  # (comments cite crate-relative paths), or a tracked directory.
  if grep -qxF "$ref" "$manifest" \
    || grep -qF -- "/$ref" "$manifest" \
    || grep -qF -- "/$ref" "$manifest.dirs" \
    || grep -qxF "$ref" "$manifest.dirs"; then
    continue
  fi

  # Unresolved. Look for evidence it once lived here; without it, this is not a
  # path into this repo and not ours to police.
  hint=""
  case "$ref" in
    */)
      # Only a same-named directory elsewhere counts. A trailing slash in a
      # comment is usually a runtime directory (`artifacts/_context/`, a
      # worktree layout), and guessing that it "collapsed into `<name>.rs`"
      # matches any unrelated module of that name — noise, not a finding.
      name="$(basename "$ref")"
      if grep -qE -- "/$name/" "$manifest"; then
        hint="a directory of that name exists elsewhere: $(grep -oE -- "[^ ]*/$name/" "$manifest" | sort -u | head -1)"
      fi
      ;;
    *.rs)
      stem="${ref%.rs}"
      base="$(basename "$stem")"
      if grep -qE -- "/$stem/" "$manifest"; then
        hint="split into a directory; the path is now \`$stem/\`"
      elif grep -qE -- "/$base\.rs$" "$manifest"; then
        hint="moved; now at $(grep -oE -- "[^ ]*/$base\.rs$" "$manifest" | sort -u | head -1)"
      fi
      ;;
    *)
      base="$(basename "$ref")"
      if grep -qE -- "/$base$" "$manifest"; then
        hint="moved; now at $(grep -E -- "/$base$" "$manifest" | sort -u | head -1)"
      fi
      ;;
  esac
  [ -n "$hint" ] || continue

  line="$(grep -nF "\`$ref\`" "$file" 2>/dev/null | head -1 | cut -d: -f1 || true)"
  echo "  $file:${line:-?}  ->  \`$ref\`"
  echo "      $hint"
  fail=1
done < <(sort -u "$manifest.refs")

if [ "$fail" = 1 ]; then
  echo
  echo "  ^ comments cite source paths that no longer exist. Update them to the"
  echo "    current path — a pointer at a deleted file is worse than no pointer."
  echo
fi

# ── Gate 2: duplicated comment blocks ────────────────────────────────────────

if [ "${DOC_REFS_DUPES:-1}" != "0" ]; then
  # Emit every run of >=3 consecutive comment lines, normalized (marker and
  # indentation stripped), keyed by content, tagged with its file.
  : >"$manifest.blocks"
  while IFS= read -r file; do
    awk -v f="$file" '
      /^[ \t]*\/\// {
        line = $0
        sub(/^[ \t]*\/\/[\/!]?[ \t]?/, "", line)
        gsub(/[ \t]+$/, "", line)
        if (line == "") { flush(); next }
        buf[n++] = line
        next
      }
      { flush() }
      END { flush() }
      function flush(  i, j, key) {
        # Every 3-line window of the block, so a shared paragraph is caught even
        # when the surrounding lines differ.
        for (i = 0; i + 2 < n; i++) {
          key = buf[i] " | " buf[i+1] " | " buf[i+2]
          print key "\t" f
        }
        n = 0
        delete buf
      }
    ' "$file"
  done < <(sources) >>"$manifest.blocks"

  dupes="$(
    sort -u "$manifest.blocks" \
      | awk -F'\t' '{ c[$1]++; where[$1] = where[$1] " " $2 }
                     END { for (k in c) if (c[k] > 1) printf "%d\t%s\t%s\n", c[k], k, where[k] }' \
      | sort -rn
  )"

  if [ -n "$dupes" ]; then
    echo "$dupes" | while IFS=$'\t' read -r count text where; do
      echo "  x$count  ${text:0:96}"
      for w in $where; do echo "        $w"; done
    done
    echo
    echo "  ^ identical comment blocks in more than one file. If it states a"
    echo "    repo-wide rule, cite AGENTS.md instead of copying it; if it"
    echo "    describes shared behaviour, document it once on the shared item."
    echo
    fail=1
  fi
fi

if [ "$fail" != 0 ]; then
  exit 1
fi

echo "doc references resolve; no duplicated comment blocks"
