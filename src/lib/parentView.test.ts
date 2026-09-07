// `parentView` is the fallback the Back control uses when there is no history
// to pop — a deep link, or the first screen of a session. It is not what Back
// normally does; a test that treated it as such would be asserting the very
// hard-coded-destination behaviour this module replaced.

import { describe, expect, it } from 'vitest';

import type { AppView } from '../types';
import { describeView, parentView } from './parentView';

describe('parentView', () => {
  it('returns a root unchanged, so Back has nothing to point at', () => {
    const home: AppView = { kind: 'home' };
    expect(parentView(home)).toBe(home);
    const empty: AppView = { kind: 'empty-state' };
    expect(parentView(empty)).toBe(empty);
  });

  it('sends a feature-scoped editor up to that feature', () => {
    expect(
      parentView({
        kind: 'editor',
        featureId: 'f-1',
        featureTitle: 'A',
        editorContext: {
          machineId: 'local',
          worktreePath: '/wt',
          branch: 'b',
          defaultBranch: 'main',
        },
      }),
    ).toEqual({ kind: 'detail', featureId: 'f-1', featureTitle: 'A' });
  });

  // The Ask canvas node's "open in editor" resolves a project checkout rather
  // than a Feature, so there is no detail view to return to. Going home beats
  // fabricating a feature id that resolves to nothing.
  it('sends an editor with no feature up to the project, not to a bogus feature', () => {
    const editorContext = {
      machineId: 'local',
      worktreePath: '/wt',
      branch: 'b',
      defaultBranch: 'main',
    };
    expect(parentView({ kind: 'editor', editorContext })).toEqual({ kind: 'home' });
    expect(
      parentView({ kind: 'editor', editorContext, featureTitle: 'orphaned title with no id' }),
    ).toEqual({ kind: 'home' });
  });

  it('returns a discovery to the tab it was opened from, not to the default one', () => {
    expect(
      parentView({ kind: 'discovery', discoveryId: 'dsc-1', discoveryTitle: 'A discovery' }),
    ).toEqual({ kind: 'home', section: 'discovery' });
  });

  it('returns the workflow editor to the workflow list', () => {
    expect(parentView({ kind: 'workflow-editor', workflowId: 'wf-1' })).toEqual({
      kind: 'workflows',
    });
  });
});

describe('describeView', () => {
  it('names the destination rather than the action', () => {
    expect(describeView({ kind: 'detail', featureId: 'f-1', featureTitle: 'Add a formatter' }))
      .toBe('Add a formatter');
    expect(describeView({ kind: 'home', section: 'discovery' })).toBe('the project');
  });

  // The title is rendered into "Back to …", so an untitled row must not produce
  // "Back to " with nothing after it.
  it('never returns an empty string for a record with no title yet', () => {
    expect(describeView({ kind: 'detail', featureId: 'f-1', featureTitle: '' })).toBe('the feature');
    expect(describeView({ kind: 'discovery', discoveryId: 'd', discoveryTitle: '' }))
      .toBe('the discovery');
  });
});
