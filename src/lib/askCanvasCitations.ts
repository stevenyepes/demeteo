import type { CanvasNode } from '../types';

/** Titles shorter than this match too much to mean anything — `Gate`, `Sync`
 *  and `Run` are all real node titles, and a bare substring test lights their
 *  ring on almost every answer. A path has no such problem: it is specific by
 *  construction, so it is matched at any length. */
const MIN_TITLE_LEN = 4;

/**
 * Whether `needle` appears in `haystack` as a whole token rather than inside
 * a longer word — `Gate` must not match "gateway", and `Sync` must not match
 * "asynchronous". Both sides are already lowercased by the callers.
 *
 * Built by hand rather than with `\b`, because the interesting needles end in
 * characters `\b` treats as boundaries themselves (`git_ops::scope`,
 * `driver.rs`): `\b` after `.rs` would happily match inside `driver.rsx`.
 */
function containsToken(haystack: string, needle: string): boolean {
  if (needle.length === 0) return false;
  let from = 0;
  for (;;) {
    const at = haystack.indexOf(needle, from);
    if (at === -1) return false;
    const before = at === 0 ? '' : haystack[at - 1];
    const after = haystack[at + needle.length] ?? '';
    if (!isWordish(before) && !isWordish(after)) return true;
    from = at + 1;
  }
}

function isWordish(ch: string): boolean {
  return ch.length === 1 && /[a-z0-9_]/.test(ch);
}

function matches(haystack: string, node: Pick<CanvasNode, 'title' | 'path'>): boolean {
  const title = node.title.toLowerCase();
  if (title.length >= MIN_TITLE_LEN && containsToken(haystack, title)) return true;
  const path = node.path?.toLowerCase();
  return path !== undefined && path.length > 0 && containsToken(haystack, path);
}

/**
 * The violet 'cited' ring's only source anywhere in this feature — never a
 * stored field (Acceptance Criterion 6). Recomputed from the answer text on
 * every render instead.
 */
export function citedNodeIds(
  answerText: string,
  nodes: readonly Pick<CanvasNode, 'id' | 'title' | 'path'>[],
): Set<string> {
  const haystack = answerText.toLowerCase();
  const cited = new Set<string>();

  if (!haystack) return cited;

  for (const node of nodes) {
    if (matches(haystack, node)) cited.add(node.id);
  }

  return cited;
}

/**
 * Derives the inspector's 'What happens here' text from the turn prose
 * instead of a stored field (spec §0) — the sentence around the same
 * title-or-path match [`citedNodeIds`] uses, not the whole answer.
 *
 * The sentence is returned as prose, not as the markdown it was written in.
 * The inspector renders it into a `<p>`, so a sentence lifted verbatim out of
 * a bulleted answer arrived carrying its own `-` and backticks.
 */
export function descriptionForNode(
  answerText: string,
  node: Pick<CanvasNode, 'title' | 'path'>,
): string | null {
  if (!answerText) return null;

  for (const sentence of answerText.split(/(?<=[.!?])\s+/)) {
    if (matches(sentence.toLowerCase(), node)) {
      const plain = stripMarkdown(sentence);
      if (plain.length > 0) return plain;
    }
  }

  return null;
}

/** Inline markdown only — a sentence, never a document, so there is no block
 *  structure left to parse by the time this runs. */
function stripMarkdown(sentence: string): string {
  return sentence
    .replace(/^\s*[-*+]\s+/, '')
    .replace(/^\s*#+\s+/, '')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/[`*_]/g, '')
    .trim();
}
