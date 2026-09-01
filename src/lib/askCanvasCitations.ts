import type { CanvasNode } from '../types';

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
    const matchesTitle = node.title.length > 0 && haystack.includes(node.title.toLowerCase());
    const matchesPath = node.path !== null && node.path.length > 0 && haystack.includes(node.path.toLowerCase());
    if (matchesTitle || matchesPath) {
      cited.add(node.id);
    }
  }

  return cited;
}

/**
 * Derives the inspector's 'What happens here' text from the turn prose
 * instead of a stored field (spec §0) — the sentence around the same
 * title-or-path match `citedNodeIds` uses, not the whole answer.
 */
export function descriptionForNode(
  answerText: string,
  node: Pick<CanvasNode, 'title' | 'path'>,
): string | null {
  if (!answerText) return null;

  const title = node.title.length > 0 ? node.title.toLowerCase() : null;
  const path = node.path !== null && node.path.length > 0 ? node.path.toLowerCase() : null;

  const sentences = answerText.split(/(?<=[.!?])\s+/);

  for (const sentence of sentences) {
    const haystack = sentence.toLowerCase();
    if ((title !== null && haystack.includes(title)) || (path !== null && haystack.includes(path))) {
      return sentence.trim();
    }
  }

  return null;
}
