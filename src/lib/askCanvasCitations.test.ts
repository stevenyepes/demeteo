import { describe, expect, it } from 'vitest';

import { citedNodeIds } from './askCanvasCitations';
import type { CanvasNode } from '../types';

function node(id: string, title: string, path: string | null = null): Pick<CanvasNode, 'id' | 'title' | 'path'> {
  return { id, title, path };
}

describe('citedNodeIds', () => {
  it('includes a node whose title appears verbatim in the answer text', () => {
    const nodes = [node('a', 'Decompose Feature')];

    const cited = citedNodeIds('The Decompose Feature step runs next.', nodes);

    expect(cited.has('a')).toBe(true);
  });

  it('includes a node whose path appears in the answer text even though its title does not', () => {
    const nodes = [node('a', 'Worktree Manager', 'src-tauri/src/adapters/worktree/mod.rs')];

    const cited = citedNodeIds('See src-tauri/src/adapters/worktree/mod.rs for the details.', nodes);

    expect(cited.has('a')).toBe(true);
  });

  it('excludes a node whose title and path both appear nowhere in the answer text', () => {
    const nodes = [node('a', 'Gate & Merge', 'src-tauri/src/domain/gate.rs')];

    const cited = citedNodeIds('This answer talks about something unrelated entirely.', nodes);

    expect(cited.has('a')).toBe(false);
  });

  it('does not false-positive a node with a null path and an absent title', () => {
    const nodes = [node('a', 'Unresolved Node', null)];

    const cited = citedNodeIds('This mentions nothing about that node at all.', nodes);

    expect(cited.has('a')).toBe(false);
  });

  it('returns an empty set for empty answer text', () => {
    const nodes = [node('a', 'Decompose Feature'), node('b', 'Gate & Merge')];

    const cited = citedNodeIds('', nodes);

    expect(cited.size).toBe(0);
  });
});
