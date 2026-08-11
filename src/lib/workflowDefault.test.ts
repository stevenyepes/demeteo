/**
 * The claim: nothing here is decided by list order.
 *
 * The bug this replaces was invisible precisely because it looked like a
 * default — `workflows[0]` picks *a* pipeline every time, so the picker is
 * never empty and the launch never fails. Every case below therefore fixes the
 * list in an order where the old fall-through would have answered differently,
 * so a regression to positional selection reddens rather than passing quietly.
 */
import { describe, expect, it } from 'vitest';

import { reconcileDefaultWorkflow, resolveLaunchWorkflowId, STANDARD_STARTER_WORKFLOW_ID } from './workflowDefault';

const custom = { id: 'wf-custom' };
const other = { id: 'wf-other' };
const standard = { id: STANDARD_STARTER_WORKFLOW_ID };

describe('resolveLaunchWorkflowId', () => {
  it('takes the caller\'s requested workflow over every other tier', () => {
    expect(
      resolveLaunchWorkflowId({
        workflows: [custom, standard, other],
        requestedId: 'wf-other',
        projectDefaultId: 'wf-custom',
      }),
    ).toBe('wf-other');
  });

  it('takes the project default when no workflow was requested', () => {
    expect(
      resolveLaunchWorkflowId({
        workflows: [custom, standard, other],
        projectDefaultId: 'wf-other',
      }),
    ).toBe('wf-other');
  });

  it('falls past a requested id that no longer resolves to the project default', () => {
    expect(
      resolveLaunchWorkflowId({
        workflows: [custom, standard, other],
        requestedId: 'wf-deleted',
        projectDefaultId: 'wf-other',
      }),
    ).toBe('wf-other');
  });

  it('falls past a project default that no longer resolves to the standard starter', () => {
    expect(
      resolveLaunchWorkflowId({
        workflows: [custom, other, standard],
        projectDefaultId: 'wf-deleted',
      }),
    ).toBe(STANDARD_STARTER_WORKFLOW_ID);
  });

  it('reads an empty string as unset, never as a match for a junk-id row', () => {
    expect(
      resolveLaunchWorkflowId({
        workflows: [{ id: '' }, custom, standard],
        requestedId: '',
        projectDefaultId: '',
      }),
    ).toBe(STANDARD_STARTER_WORKFLOW_ID);
  });

  it('names the standard starter when nothing else is set', () => {
    expect(
      resolveLaunchWorkflowId({ workflows: [custom, other, standard] }),
    ).toBe(STANDARD_STARTER_WORKFLOW_ID);
  });

  it('picks the only workflow a project has, standard starter or not', () => {
    expect(resolveLaunchWorkflowId({ workflows: [custom] })).toBe('wf-custom');
  });

  it('answers null rather than guessing between several unnamed workflows', () => {
    expect(resolveLaunchWorkflowId({ workflows: [custom, other] })).toBeNull();
  });

  it('answers null for a project with no workflows at all', () => {
    expect(
      resolveLaunchWorkflowId({
        workflows: [],
        requestedId: 'wf-other',
        projectDefaultId: 'wf-custom',
      }),
    ).toBeNull();
  });
});

const WORKFLOWS = [{ id: 'wf-standard' }, { id: 'wf-fast' }];

describe('reconcileDefaultWorkflow', () => {
  it('reads a stored id the list still answers to as the selection', () => {
    expect(reconcileDefaultWorkflow('wf-fast', WORKFLOWS)).toEqual({
      selected: 'wf-fast',
      dangling: null,
    });
  });

  it('treats an absent stored value as not set', () => {
    expect(reconcileDefaultWorkflow(null, WORKFLOWS)).toEqual({ selected: '', dangling: null });
    expect(reconcileDefaultWorkflow(undefined, WORKFLOWS)).toEqual({ selected: '', dangling: null });
    expect(reconcileDefaultWorkflow('', WORKFLOWS)).toEqual({ selected: '', dangling: null });
  });

  it('degrades a stored id no workflow answers to, and reports the orphan', () => {
    expect(reconcileDefaultWorkflow('wf-deleted', WORKFLOWS)).toEqual({
      selected: '',
      dangling: 'wf-deleted',
    });
  });

  it('degrades every stored id when the project has no workflows at all', () => {
    expect(reconcileDefaultWorkflow('wf-fast', [])).toEqual({
      selected: '',
      dangling: 'wf-fast',
    });
  });
});
