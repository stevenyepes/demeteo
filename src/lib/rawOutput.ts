/**
 * Making a stored command transcript readable, decided here so a 2552-line one
 * can be asserted without rendering anything.
 *
 * `sync_sessions.raw_error` keeps a blocked sync's output **verbatim** on
 * purpose — the row is the only record of what the project's harness said, and
 * a backend that trimmed it would be choosing, once and permanently, which half
 * mattered. So the trimming is a rendering decision and lives on this side of
 * the wire, where it is reversible by a press.
 *
 * Which half matters is not a guess. `merge_gate_refusal`
 * (crates/demeteo-core/src/adapters/worktree/git_ops/sync_verify.rs) writes the
 * first lines itself — the command and its exit code — and the harness writes
 * the last, which is where every runner this project targets puts its verdict.
 * The middle is the part a scrollbar is for. That is the whole reason this is
 * head+tail rather than a leading truncation: a `<pre>` showing the first
 * screen of a check run shows warnings that are not why it failed, and the one
 * incident this was built from ended with the user reading 131 biome warnings
 * and concluding the checks had never run.
 *
 * Deliberately **not** here: any reading of `==>` stage markers. Those are one
 * repo's `scripts/checks.sh`; a project whose `default_test_command` is `make
 * test` emits nothing of the sort, and a headline that names the wrong stage is
 * worse than no headline.
 */

/** How many leading lines carry the command and its exit code. */
const HEAD_LINES = 6;

/** How many trailing lines carry the verdict. Wide enough for a Rust
 *  diagnostic with its `help:` block, which is the longest single unit a
 *  reader has to see whole. */
const TAIL_LINES = 40;

/** Below this, eliding costs a press to save less than a scroll. */
const MIN_HIDDEN_LINES = 5;

/**
 * The escape sequences a terminal consumes and a `<pre>` renders as literal
 * garbage. Assembled rather than written as a literal so the ESC byte never
 * appears in a regex source, which `noControlCharactersInRegex` denies.
 *
 * CSI (`ESC [ … final`) is what colour comes as; OSC (`ESC ] … BEL`/ST) is what
 * a harness setting the window title emits; the third arm takes the two-byte
 * escapes on their own.
 *
 * Only the real byte is matched. Caret notation (`^[[1;34m`) is what a
 * `sqlite3` dump of the same row shows, never what crosses the IPC boundary,
 * and an arm for it would eat the head of a regex character class — `^[[a-z]+$`
 * in a failing test's own output becomes `-z]+$`. Mangling the transcript a
 * user opened this pane to read is worse than the garbage it would hide.
 */
const ESC = String.fromCharCode(0x1b);
const BEL = String.fromCharCode(0x07);
const CSI_BODY = '[0-?]*[ -/]*[@-~]';
const ANSI_SEQUENCE = new RegExp(
  [
    `${ESC}\\[${CSI_BODY}`,
    `${ESC}\\][^${ESC}${BEL}]*(?:${BEL}|${ESC}\\\\)`,
    `${ESC}[@-Z\\\\-_]`,
  ].join('|'),
  'g',
);

export function stripAnsi(text: string): string {
  return text.replace(ANSI_SEQUENCE, '');
}

/**
 * A transcript that fits, or one split around what it hides.
 *
 * A union rather than an always-populated shape: the overwhelmingly common
 * `detail` is a single `fatal:` line, and `kind: 'whole'` is what keeps the
 * caller from wrapping one in an expander that has nothing to expand.
 */
export type ReadableOutput =
  | { kind: 'whole'; text: string }
  | {
      kind: 'elided';
      head: string;
      tail: string;
      hiddenLines: number;
      totalLines: number;
      full: string;
    };

export function readableOutput(raw: string): ReadableOutput {
  const text = stripAnsi(raw);
  // Trailing blank lines are a shell artefact, and counting them as content
  // spends the tail's last rows saying nothing where the verdict should be.
  const lines = text.trimEnd().split('\n');
  const hiddenLines = lines.length - HEAD_LINES - TAIL_LINES;
  if (hiddenLines < MIN_HIDDEN_LINES) return { kind: 'whole', text };
  return {
    kind: 'elided',
    head: lines.slice(0, HEAD_LINES).join('\n'),
    tail: lines.slice(lines.length - TAIL_LINES).join('\n'),
    hiddenLines,
    totalLines: lines.length,
    full: text,
  };
}
