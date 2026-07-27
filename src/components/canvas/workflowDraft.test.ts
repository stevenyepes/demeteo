/**
 * Draft autosave storage (task P3.3). The load path is the interesting half:
 * it reads data written by an *older build* of the app, so every field is
 * treated as untrusted and anything unreadable degrades to "no draft" rather
 * than to a half-populated canvas.
 */
import { afterEach, describe, expect, it } from 'vitest';

import { clearDraft, draftKey, loadDraft, saveDraft } from './workflowDraft';
import type { WorkflowDefinitionV2 } from './types';

const def: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf-d',
  name: 'Draft',
  nodes: [{ id: 'plan', type: 'agent', title: 'Plan' }],
  edges: [],
};

const draft = (over: Partial<Parameters<typeof saveDraft>[0]> = {}) => ({
  workflowId: 'wf-d',
  name: 'Draft',
  description: 'desc',
  definition: def,
  savedAt: 1_700_000_000_000,
  ...over,
});

afterEach(() => localStorage.clear());

describe('workflowDraft', () => {
  it('round-trips a draft', () => {
    saveDraft(draft());
    const loaded = loadDraft('wf-d');
    expect(loaded?.definition).toEqual(def);
    expect(loaded?.name).toBe('Draft');
    expect(loaded?.description).toBe('desc');
    expect(loaded?.savedAt).toBe(1_700_000_000_000);
  });

  it('keys drafts per workflow, with a slot for an uncreated one', () => {
    saveDraft(draft());
    saveDraft(draft({ workflowId: null, name: 'Brand new' }));

    expect(loadDraft('wf-d')?.name).toBe('Draft');
    expect(loadDraft(null)?.name).toBe('Brand new');
    // A workflow with no draft doesn't inherit someone else's.
    expect(loadDraft('wf-other')).toBeNull();
    expect(draftKey(null)).not.toBe(draftKey('wf-d'));
  });

  it('clears one slot without touching the others', () => {
    saveDraft(draft());
    saveDraft(draft({ workflowId: null }));
    clearDraft('wf-d');
    expect(loadDraft('wf-d')).toBeNull();
    expect(loadDraft(null)).not.toBeNull();
  });

  it('ignores a draft written in an older format', () => {
    localStorage.setItem(
      draftKey('wf-d'),
      JSON.stringify({ ...draft(), format: 0 }),
    );
    expect(loadDraft('wf-d')).toBeNull();
  });

  it('ignores unparseable storage and a draft with no graph', () => {
    localStorage.setItem(draftKey('wf-d'), 'not json{');
    expect(loadDraft('wf-d')).toBeNull();

    localStorage.setItem(
      draftKey('wf-d'),
      JSON.stringify({ format: 1, workflowId: 'wf-d', definition: { nodes: 'nope' } }),
    );
    expect(loadDraft('wf-d')).toBeNull();
  });

  it('tolerates missing meta fields', () => {
    localStorage.setItem(
      draftKey('wf-d'),
      JSON.stringify({ format: 1, workflowId: 'wf-d', definition: def }),
    );
    const loaded = loadDraft('wf-d');
    expect(loaded?.name).toBe('');
    expect(loaded?.savedAt).toBe(0);
  });
});
