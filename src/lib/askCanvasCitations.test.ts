import { describe, expect, it } from 'vitest';

import { citedNodeIds, descriptionForNode } from './askCanvasCitations';
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

  it('does not light a short title that only appears inside a longer word', () => {
    const nodes = [node('a', 'Gate'), node('b', 'Sync')];

    const cited = citedNodeIds(
      'The gateway is asynchronous, so nothing here names either node.',
      nodes,
    );

    expect(cited.size).toBe(0);
  });

  it('still matches a whole-token occurrence of a title that short', () => {
    const nodes = [node('a', 'Gate')];

    expect(citedNodeIds('Work stops at the Gate until a person decides.', nodes).has('a')).toBe(
      true,
    );
  });

  it('matches a path even where a word boundary would not fall', () => {
    // `.rs` ends in a non-word character, so `\b` after it would happily
    // match inside `driver.rsx` — hence the hand-rolled token test.
    const nodes = [node('a', 'Step execution', 'adapters/driver.rs')];

    expect(citedNodeIds('It lives in adapters/driver.rsx now.', nodes).has('a')).toBe(false);
    expect(citedNodeIds('It lives in `adapters/driver.rs` now.', nodes).has('a')).toBe(true);
  });

  it('returns an empty set for empty answer text', () => {
    const nodes = [node('a', 'Decompose Feature'), node('b', 'Gate & Merge')];

    const cited = citedNodeIds('', nodes);

    expect(cited.size).toBe(0);
  });
});

describe('descriptionForNode', () => {
  it('returns the sentence containing a title match', () => {
    const target = node('a', 'Decompose Feature');

    const description = descriptionForNode(
      'First the workflow starts. The Decompose Feature step runs next. Then it gates for approval.',
      target,
    );

    expect(description).toBe('The Decompose Feature step runs next.');
  });

  it('returns the sentence containing a path match when the title does not appear', () => {
    const target = node('a', 'Worktree Manager', 'src-tauri/src/adapters/worktree/mod.rs');

    const description = descriptionForNode(
      'Setup happens first. See src-tauri/src/adapters/worktree/mod.rs for the details. Cleanup happens last.',
      target,
    );

    expect(description).toBe('See src-tauri/src/adapters/worktree/mod.rs for the details.');
  });

  it('returns null when neither the title nor the path appears in the prose', () => {
    const target = node('a', 'Gate & Merge', 'src-tauri/src/domain/gate.rs');

    const description = descriptionForNode('This answer talks about something unrelated entirely.', target);

    expect(description).toBeNull();
  });

  it('returns the sentence as prose, not as the markdown it was written in', () => {
    const target = node('a', 'ExecutionPort');

    const description = descriptionForNode(
      '- **ExecutionPort** (`ports/execution.rs`) is the one behavioural contract.',
      target,
    );

    expect(description).toBe('ExecutionPort (ports/execution.rs) is the one behavioural contract.');
  });

  it('returns only the matching sentence, not the whole multi-sentence answer', () => {
    const target = node('a', 'Decompose Feature');

    const description = descriptionForNode(
      'The workflow begins with setup. The Decompose Feature step splits work into tickets. Finally it gates for approval.',
      target,
    );

    expect(description).not.toContain('Finally it gates for approval.');
    expect(description).toBe('The Decompose Feature step splits work into tickets.');
  });
});
