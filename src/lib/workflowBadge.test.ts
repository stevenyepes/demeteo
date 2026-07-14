// Tests for the workflow badge lookup used by the pipeline cards in ProjectHome.
//
// Was `tests/repro/workflow-indicator.mjs`, which re-implemented both functions
// inline and warned it had to be "kept in sync with the production code". The
// logic now lives in `src/lib/workflowBadge.ts` and ProjectHome imports it, so
// this exercises the real thing.

import { describe, expect, it } from 'vitest';

import { buildWorkflowById, classifyWorkflowBadge } from './workflowBadge';

const WORKFLOWS = [
  { id: 'wf-bugfix', name: 'Bugfix Pipeline', is_starter: true },
  { id: 'wf-feature', name: 'Standard Feature Pipeline', is_starter: false },
  { id: 'wf-research', name: 'Research Consulting', is_starter: false },
];

const lookup = buildWorkflowById(WORKFLOWS);

describe('classifyWorkflowBadge', () => {
  it('names a starter workflow', () => {
    expect(classifyWorkflowBadge({ workflow_id: 'wf-bugfix' }, lookup)).toEqual({
      variant: 'known',
      name: 'Bugfix Pipeline',
      is_starter: true,
    });
  });

  it('names a custom workflow', () => {
    expect(classifyWorkflowBadge({ workflow_id: 'wf-feature' }, lookup)).toEqual({
      variant: 'known',
      name: 'Standard Feature Pipeline',
      is_starter: false,
    });
  });

  // The original bug: a miss rendered a violet badge reading "undefined" as if
  // a real workflow had matched.
  it.each([
    ['undefined', undefined],
    ['null', null],
    ['an empty string', ''],
    ['an id that no longer exists', 'wf-deleted'],
  ])('falls back to the muted badge for %s', (_label, workflow_id) => {
    expect(classifyWorkflowBadge({ workflow_id }, lookup)).toEqual({ variant: 'fallback' });
  });
});

describe('buildWorkflowById', () => {
  // The other bug: a positional match relabelled cards with the wrong workflow.
  it('keys by id, so a refetch that shrinks or renames the list stays correct', () => {
    const refetched = buildWorkflowById([
      { id: 'wf-bugfix', name: 'Bugfix v2 (renamed)', is_starter: true },
    ]);

    expect(classifyWorkflowBadge({ workflow_id: 'wf-bugfix' }, refetched)).toMatchObject({
      variant: 'known',
      name: 'Bugfix v2 (renamed)',
    });
    expect(classifyWorkflowBadge({ workflow_id: 'wf-feature' }, refetched)).toEqual({
      variant: 'fallback',
    });
  });

  it('drops entries with a missing or non-string id', () => {
    const dirty = buildWorkflowById([
      null,
      undefined,
      { name: 'no id' },
      { id: 123, name: 'numeric id' },
      { id: '', name: 'empty id' },
      { id: 'wf-ok', name: 'OK', is_starter: false },
    ]);

    expect([...dirty.keys()]).toEqual(['wf-ok']);
  });
});
