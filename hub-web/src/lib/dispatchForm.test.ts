import { describe, expect, it } from 'vitest';

import { EFFORT_LEVELS } from '../../../src/lib/effortLevels';
import {
  agentChoicesFor,
  buildDispatchRequest,
  type DispatchForm,
  definitionOf,
  dispatchBlocker,
  dispatchSources,
  effortsFor,
  isOverridable,
  modelChoicesFor,
  type Prefill,
  prefillFor,
  parseMaxCostCents,
  parseWallClockSecs,
  runnerProjectProblem,
  toStepOverrides,
  usdOfCents,
} from './dispatchForm';
import type { FleetInstance, HarnessCatalog, InstanceDetail, ProjectSummary, WorkflowChoice } from './fleet';

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
  kind: 'desktop',
  readGranted: true,
  projects: [{ id: 'p-1', name: 'demeteo', remoteUrl: null, providerKind: null }],
  liveRuns: [],
  harnesses: null,
  harnessCatalog: null,
  workflows: [{ workflowId: 'wf-1', version: 3, name: 'Default', graph: null }],
  runShape: [],
};

const CODEX: HarnessCatalog = {
  kind: 'codex',
  models: [{ value: 'gpt-5.5', name: 'GPT-5.5' }],
  effortLevels: ['low', 'medium'],
  source: 'bundled',
  probedAt: 1_700_000_000,
};

const OPENCODE: HarnessCatalog = {
  kind: 'opencode',
  models: [{ value: 'anthropic/claude-opus', name: 'Claude Opus' }],
  effortLevels: null,
  source: 'probed',
  probedAt: 1_700_000_000,
};

const CATALOGUED: InstanceDetail = {
  ...INSTANCE,
  harnesses: ['claude-code', 'codex', 'opencode'],
  harnessCatalog: [CODEX, OPENCODE],
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

describe('agentChoicesFor', () => {
  it('offers the harnesses the instance found installed', () => {
    expect(agentChoicesFor(CATALOGUED)).toEqual(['codex', 'opencode']);
  });

  it('offers none when the instance probed and found none installed', () => {
    expect(agentChoicesFor({ ...CATALOGUED, harnessCatalog: [] })).toEqual([]);
  });

  it('falls back to the registered harnesses, never a test runtime, from an instance with no catalog', () => {
    expect(agentChoicesFor({ ...INSTANCE, harnesses: ['noop', 'claude-code', 'stub', 'pi'] })).toEqual([
      'claude-code',
      'pi',
    ]);
  });

  it('falls back to every kind this build knows from an instance reporting neither', () => {
    expect(agentChoicesFor(INSTANCE)).toEqual(['claude-code', 'codex', 'opencode', 'hermes']);
  });
});

describe('modelChoicesFor', () => {
  it("lists the kind's models and where they came from", () => {
    expect(modelChoicesFor(CATALOGUED, 'codex')).toEqual({ models: CODEX.models, source: 'bundled' });
  });

  it('is free text for a kind the catalog has no entry for, or with no catalog at all', () => {
    expect(modelChoicesFor(CATALOGUED, 'claude-code')).toBeNull();
    expect(modelChoicesFor(CATALOGUED, '')).toBeNull();
    expect(modelChoicesFor(INSTANCE, 'codex')).toBeNull();
  });

  it('is an empty list, not free text, for an entry that lists no model', () => {
    const empty = { ...CATALOGUED, harnessCatalog: [{ ...CODEX, models: [] }] };
    expect(modelChoicesFor(empty, 'codex')).toEqual({ models: [], source: 'bundled' });
  });
});

describe('effortsFor', () => {
  it("takes the instance's levels for a kind it reports them for", () => {
    expect(effortsFor(CATALOGUED, 'codex')).toEqual(['low', 'medium']);
  });

  it("falls back to this build's table where the instance gives none", () => {
    expect(effortsFor(CATALOGUED, 'opencode')).toEqual(EFFORT_LEVELS);
    expect(effortsFor(INSTANCE, 'codex')).toEqual(['low', 'medium', 'high', 'xhigh']);
    expect(effortsFor(CATALOGUED, 'hermes')).toEqual([]);
  });
});

describe('prefillFor', () => {
  const UNFILLED: Prefill = { agentKind: '', model: '', effort: '', workflowId: '', maxCostUsd: '' };
  const withDefaults = (agentKind: string): InstanceDetail => ({
    ...INSTANCE,
    runShape: [
      { projectId: 'p-1', agentKind, model: 'm', effort: 'high', workflowId: null, maxBudgetCents: 500 },
    ],
  });

  it("fills the project's default agent when the target can run it, clamping effort to the target's levels", () => {
    const filled = prefillFor(withDefaults('codex'), 'p-1', UNFILLED, new Set(), CATALOGUED);
    expect(filled).toMatchObject({ agentKind: 'codex', model: 'm', effort: 'medium' });
  });

  it('leaves the agent blank when the target does not offer it, judged on the target alone', () => {
    const source = { ...withDefaults('claude-code'), harnessCatalog: [{ ...CODEX, kind: 'claude-code' }] };
    const filled = prefillFor(source, 'p-1', UNFILLED, new Set(), CATALOGUED);
    expect(filled.agentKind).toBe('');
  });
});

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
        sourceInstanceId: 'src-1',
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
        'source_instance_id',
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
      source_instance_id: 'src-1',
    });
  });
});

function project(remoteUrl: string | null, providerKind: string | null): ProjectSummary {
  return { id: 'p', name: 'widgets', remoteUrl, providerKind };
}

describe('runnerProjectProblem', () => {
  it('offers a GitHub project whose remote is https://host/owner/repo, a nested group included', () => {
    expect(runnerProjectProblem(project('https://github.com/acme/widgets', 'github'))).toBeNull();
    expect(runnerProjectProblem(project('https://ghe.example.com/org/team/widgets', 'github'))).toBeNull();
  });

  it('refuses a project with no Git host, as the Hub would', () => {
    expect(runnerProjectProblem(project('https://github.com/acme/widgets', null))).toMatch(/no Git host/);
    expect(runnerProjectProblem(project('https://github.com/acme/widgets', ''))).toMatch(/no Git host/);
  });

  it('refuses GitLab, which the Hub accepts but no runner can fetch a token for', () => {
    expect(runnerProjectProblem(project('https://gitlab.com/acme/widgets', 'gitlab'))).toMatch(/GitLab/);
  });

  it.each([
    null,
    'git@github.com:acme/widgets.git',
    'http://github.com/acme/widgets',
    'https://github.com/widgets',
    'https:///acme/widgets',
    'https://github.com/acme//widgets',
    'https://github.com/acme/widgets/',
    'https://user@github.com/acme/widgets',
    'https://github.com/acme/wid gets',
    'https://github.com/acme/wid\u0085gets',
  ])('refuses the remote %j, whose shape the Hub would not read', (remoteUrl) => {
    expect(runnerProjectProblem(project(remoteUrl, 'github'))).toMatch(/remote/);
  });
});

describe('dispatchSources', () => {
  const desktop: FleetInstance = {
    id: 'd-1',
    kind: 'desktop',
    label: 'laptop',
    present: true,
    os: 'linux',
    arch: 'x86_64',
    version: '1.0.0',
    scopes: ['read'],
    readGranted: true,
    lastSeen: null,
    liveRuns: 0,
    needsYou: [],
    harnesses: null,
    projectCount: 2,
  };

  it('keeps only desktops reporting read with projects, whatever their spend', () => {
    const fleet: FleetInstance[] = [
      desktop,
      { ...desktop, id: 'd-spend', scopes: ['read', 'spend'] },
      { ...desktop, id: 'r-1', kind: 'runner' },
      { ...desktop, id: 'd-no-read', readGranted: false },
      { ...desktop, id: 'd-empty', projectCount: 0 },
      { ...desktop, id: 'd-unknown-kind', kind: null },
    ];
    expect(dispatchSources(fleet).map((instance) => instance.id)).toEqual(['d-1', 'd-spend']);
  });
});
