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
