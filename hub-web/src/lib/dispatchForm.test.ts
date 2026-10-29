import { describe, expect, it } from 'vitest';

import {
  buildDispatchRequest,
  type DispatchForm,
  definitionOf,
  dispatchBlocker,
  isOverridable,
  parseMaxCostCents,
  parseWallClockSecs,
  toStepOverrides,
  usdOfCents,
} from './dispatchForm';
import type { InstanceDetail, WorkflowChoice } from './fleet';

const INSTANCE: InstanceDetail = {
  id: '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21',
  present: true,
  connectedAt: 1_700_000_000,
  lastSeen: 1_700_000_100,
  installId: null,
  os: 'linux',
  arch: 'x86_64',
  version: '1.0.0',
  scopes: ['read', 'spend'],
  readGranted: true,
  projects: [{ id: 'p-1', name: 'demeteo', remoteUrl: null }],
  liveRuns: [],
  harnesses: null,
  workflows: [{ workflowId: 'wf-1', version: 3, name: 'Default', graph: null }],
  runShape: [],
};

const CHOICE: WorkflowChoice = {
  workflowId: 'wf-1',
  version: 3,
  name: 'Default',
  graph: {
    nodes: [
      { id: 's-implement', type: 'agent', title: 'Implement', position: { x: 0, y: 0 } },
      { id: 's-gate', type: 'gate', title: 'Review', position: null },
    ],
    edges: [{ from: 's-implement', to: 's-gate' }],
  },
};

const FORM: DispatchForm = {
  projectId: 'p-1',
  workflowId: 'wf-1',
  title: 'Add a login form',
  description: 'With a remember-me box',
  agentKind: '',
  model: '',
  effort: null,
  overrides: {},
  maxBudgetCents: 500,
  maxWallClockSecs: undefined,
};

describe('parseMaxCostCents', () => {
  it.each(['', '   ', '-1', '0', '0.00', '1e2', 'abc', '1.234', '1.', '.5', '+1', '0x10', '1,5', '12 5'])(
    'refuses %j',
    (text) => {
      expect(parseMaxCostCents(text)).toMatchObject({ ok: false });
    },
  );

  it.each([
    ['0.29', 29],
    ['12', 1200],
    ['12.5', 1250],
    ['0.01', 1],
    [' 4.10 ', 410],
    ['1.15', 115],
  ])('reads %j as %d cents', (text, cents) => {
    expect(parseMaxCostCents(text)).toEqual({ ok: true, value: cents });
  });

  it('refuses an amount past the safe-integer range of cents', () => {
    const dollars = String(Math.floor(Number.MAX_SAFE_INTEGER / 100) + 1);
    expect(parseMaxCostCents(dollars)).toMatchObject({ ok: false });
    expect(parseMaxCostCents('99999999999999999999')).toMatchObject({ ok: false });
  });

  it('keeps the largest amount that fits', () => {
    const cents = Number.MAX_SAFE_INTEGER;
    const text = `${Math.floor(cents / 100)}.${String(cents % 100).padStart(2, '0')}`;
    expect(parseMaxCostCents(text)).toEqual({ ok: true, value: cents });
  });

  it('says why a blank cap is refused', () => {
    const result = parseMaxCostCents('');
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toMatch(/required/i);
  });
});

describe('usdOfCents', () => {
  it.each([
    [5, '0.05'],
    [100, '1.00'],
    [1205, '12.05'],
    [29, '0.29'],
    [Number.MAX_SAFE_INTEGER, '90071992547409.91'],
  ])('writes %d cents as %j, which parses back to the same cents', (cents, text) => {
    expect(usdOfCents(cents)).toBe(text);
    expect(parseMaxCostCents(text)).toEqual({ ok: true, value: cents });
  });

  it.each([0, -100, 12.5, Number.NaN])('leaves %d blank, which max cost refuses as required', (cents) => {
    expect(usdOfCents(cents)).toBe('');
  });
});

describe('parseWallClockSecs', () => {
  it('reads blank as no cap', () => {
    expect(parseWallClockSecs('')).toEqual({ ok: true, value: undefined });
    expect(parseWallClockSecs('  ')).toEqual({ ok: true, value: undefined });
  });

  it('reads whole minutes as seconds', () => {
    expect(parseWallClockSecs('90')).toEqual({ ok: true, value: 5400 });
  });

  it.each(['0', '-5', '1.5', '1e2', 'abc'])('refuses %j', (text) => {
    expect(parseWallClockSecs(text)).toMatchObject({ ok: false });
  });
});

describe('toStepOverrides', () => {
  it('drops all-empty entries, keeps partial ones in order and omits unset fields', () => {
    expect(
      toStepOverrides({
        's-b': { agentKind: 'codex' },
        's-empty': {},
        's-blank': { agentKind: '', model: '' },
        's-a': { agentKind: 'claude-code', model: 'sonnet', effort: 'high' },
        's-c': { effort: 'low' },
      }),
    ).toEqual([
      { step_id: 's-b', agent_kind: 'codex' },
      { step_id: 's-a', agent_kind: 'claude-code', model: 'sonnet', effort: 'high' },
      { step_id: 's-c', effort: 'low' },
    ]);
  });

  it('emits nothing for an empty map', () => {
    expect(toStepOverrides({})).toEqual([]);
  });
});

describe('isOverridable', () => {
  it.each([
    ['agent', true],
    ['sequence', true],
    ['gate', false],
    ['sync', false],
    ['finalize', false],
    ['command', false],
  ])('%s → %s', (type, expected) => {
    expect(isOverridable(type)).toBe(expected);
  });
});

describe('dispatchBlocker', () => {
  it('is no-read while projects or workflows are withheld', () => {
    expect(dispatchBlocker({ ...INSTANCE, scopes: ['spend'], projects: null, workflows: null })).toBe('no-read');
    expect(dispatchBlocker({ ...INSTANCE, workflows: null })).toBe('no-read');
    expect(dispatchBlocker({ ...INSTANCE, projects: null })).toBe('no-read');
  });

  it('is no-spend with read but without spend', () => {
    expect(dispatchBlocker({ ...INSTANCE, scopes: ['read', 'gates'] })).toBe('no-spend');
  });

  it('is no-snapshot when projects or workflows are empty', () => {
    expect(dispatchBlocker({ ...INSTANCE, projects: [] })).toBe('no-snapshot');
    expect(dispatchBlocker({ ...INSTANCE, workflows: [] })).toBe('no-snapshot');
  });

  it('does not block an offline instance with spend, which queues', () => {
    expect(dispatchBlocker({ ...INSTANCE, present: false, connectedAt: null })).toBeNull();
  });
});

describe('definitionOf', () => {
  it('carries the nodes, edges and name with no config on any node', () => {
    const definition = definitionOf(CHOICE);
    expect(definition).not.toBeNull();
    expect(definition?.name).toBe('Default');
    expect(definition?.id).toBe('wf-1');
    expect(definition?.edges).toEqual([{ from: 's-implement', to: 's-gate' }]);
    expect(definition?.nodes).toEqual([
      { id: 's-implement', type: 'agent', title: 'Implement', position: { x: 0, y: 0 } },
      { id: 's-gate', type: 'gate', title: 'Review', position: null },
    ]);
    for (const node of definition?.nodes ?? []) expect(Object.keys(node)).not.toContain('config');
  });

  it('is null for a workflow with no graph', () => {
    expect(definitionOf({ ...CHOICE, graph: null })).toBeNull();
  });
});

describe('buildDispatchRequest', () => {
  it('emits only the required DispatchBody keys when nothing optional is set', () => {
    const body = buildDispatchRequest(FORM, []);
    expect(Object.keys(body).sort()).toEqual(
      [
        'attachment_ids',
        'description',
        'max_budget_cents',
        'project_id',
        'step_overrides',
        'title',
        'workflow_id',
      ].sort(),
    );
    expect(body.max_budget_cents).toBe(500);
    expect(Number.isInteger(body.max_budget_cents)).toBe(true);
  });

  it('emits every DispatchBody key, and nothing else, when every field is set', () => {
    const body = buildDispatchRequest(
      {
        ...FORM,
        agentKind: 'claude-code',
        model: 'sonnet',
        effort: 'medium',
        overrides: { 's-implement': { model: 'opus' }, 's-gate': {} },
        maxWallClockSecs: 5400,
      },
      ['att-1', 'att-2'],
    );
    expect(Object.keys(body).sort()).toEqual(
      [
        'agent_kind',
        'attachment_ids',
        'description',
        'effort',
        'max_budget_cents',
        'max_wall_clock_secs',
        'model',
        'project_id',
        'step_overrides',
        'title',
        'workflow_id',
      ].sort(),
    );
    expect(body).toEqual({
      project_id: 'p-1',
      workflow_id: 'wf-1',
      title: 'Add a login form',
      description: 'With a remember-me box',
      agent_kind: 'claude-code',
      model: 'sonnet',
      effort: 'medium',
      step_overrides: [{ step_id: 's-implement', model: 'opus' }],
      max_budget_cents: 500,
      max_wall_clock_secs: 5400,
      attachment_ids: ['att-1', 'att-2'],
    });
  });
});
