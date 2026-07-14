// Pure-function tests for the Create-Project wizard.
//
// The two regressions this file is contractually bound to catch:
//   1. STEP_ORDER must contain EXACTLY seven entries (C-3 — extra
//      strategy/workflow/launching screens are forbidden inside the wizard).
//   2. goBack must be derived from `state.history`, never from a raw index into
//      STEP_ORDER (C-4 — the counter-based goBack silently re-entered
//      auto-progressed screens).

import { describe, expect, it } from 'vitest';

import { BootstrapStep, STEP_ORDER } from './types';
import { canRewind, buildCommitPayload } from './components/wizard/CreateProjectWizard';
import { canGoBackFromHistory, rewindHistory } from './components/wizard/WizardShell';
import type { BootstrapState, CreateProjectStepPayload } from './types';

const EXPECTED_ORDER: ReadonlyArray<BootstrapStep> = [
  BootstrapStep.Name,
  BootstrapStep.Provider,
  BootstrapStep.Group,
  BootstrapStep.Machine,
  BootstrapStep.Agent,
  BootstrapStep.Model,
  BootstrapStep.Description,
];

// AC-5: the wizard renders exactly seven one-decision-per-screen steps.
describe('STEP_ORDER', () => {
  it('contains exactly seven steps in the canonical order', () => {
    expect(STEP_ORDER).toEqual(EXPECTED_ORDER);
  });

  // The kebab-case slugs are the IPC contract — the Rust
  // `CreateProjectStepPayload` matches on them via `expected_step()`.
  it('keeps the discriminant slugs stable across the IPC boundary', () => {
    expect([...STEP_ORDER]).toEqual([
      'name',
      'provider',
      'group',
      'machine',
      'agent',
      'model',
      'description',
    ]);
  });
});

describe('canRewind / canGoBackFromHistory', () => {
  it('cannot rewind out of the initial state', () => {
    expect(canRewind({ step: BootstrapStep.Name, history: [BootstrapStep.Name] })).toBe(false);
    expect(canGoBackFromHistory([BootstrapStep.Name])).toBe(false);
  });

  it('can rewind after a single advance', () => {
    expect(
      canRewind({
        step: BootstrapStep.Provider,
        history: [BootstrapStep.Name, BootstrapStep.Provider],
      }),
    ).toBe(true);
    expect(canGoBackFromHistory([BootstrapStep.Name, BootstrapStep.Provider])).toBe(true);
  });
});

// C-4 regression. The previous implementation rewound via
// `STEP_ORDER.indexOf(step) - 1`, which silently re-entered auto-progressed
// screens. The fix pops the history stack instead, so an auto-progressed
// Provider step (recorded in history) is still rewindable and the caller can
// keep rewinding past it.
describe('rewindHistory', () => {
  const history: ReadonlyArray<BootstrapStep> = [
    BootstrapStep.Name,
    BootstrapStep.Provider, // auto-progressed
    BootstrapStep.Group, // auto-progressed
    BootstrapStep.Machine,
  ];

  it('pops the history stack one auto-progressed entry at a time', () => {
    expect(rewindHistory(history)).toBe(BootstrapStep.Group);
    expect(rewindHistory(history.slice(0, 3))).toBe(BootstrapStep.Provider);
    expect(rewindHistory(history.slice(0, 2))).toBe(BootstrapStep.Name);
  });

  it('returns null once the history bottoms out at Name', () => {
    expect(rewindHistory(history.slice(0, 1))).toBeNull();
  });
});

describe('buildCommitPayload', () => {
  const draft = {
    name: 'billing-service',
    providerId: 'prov-1',
    providerKind: 'github',
    providerHost: 'github.com',
    namespaceId: 'my-org',
    namespaceKind: 'org' as const,
    namespaceName: 'My Org',
    machineKind: 'remote' as const,
    machineId: 'machine-1',
    keyPassphrase: 'should-not-leak',
    agentKind: 'opencode',
    model: 'anthropic/claude-sonnet-4',
    effort: 'xhigh' as const,
    title: 'Implement billing service',
    description: 'A billing service written in Rust.',
    visibility: 'private' as const,
  };

  it('produces the wire shape the Rust CreateProjectStepPayload::Commit expects', () => {
    const expected: CreateProjectStepPayload = {
      step: 'commit',
      title: 'Implement billing service',
      description: 'A billing service written in Rust.',
      visibility: 'private',
      name: 'billing-service',
      provider_id: 'prov-1',
      provider_kind: 'github',
      provider_host: 'github.com',
      namespace_id: 'my-org',
      namespace_kind: 'org',
      namespace_name: 'My Org',
      machine_kind: 'remote',
      machine_id: 'machine-1',
      agent_kind: 'opencode',
      model: 'anthropic/claude-sonnet-4',
      effort: 'xhigh',
    };

    expect(buildCommitPayload(draft)).toEqual(expected);
  });

  // The Rust port doesn't carry the passphrase; the wizard writes it to the
  // keyring via a separate `set_machine_secret` IPC before bootstrap runs.
  it('never leaks the key passphrase into the commit payload', () => {
    expect(buildCommitPayload(draft)).not.toHaveProperty('keyPassphrase');
  });

  // OQ-5: a blank title falls back to the project name.
  it('falls back to the project name when the title is blank', () => {
    const payload = buildCommitPayload({
      ...draft,
      namespaceId: 'ns',
      namespaceKind: 'personal' as const,
      namespaceName: 'me',
      machineKind: 'local' as const,
      machineId: null,
      keyPassphrase: '',
      model: 'm',
      title: '',
      description: 'Build the billing service.',
      visibility: 'public' as const,
    });

    expect(payload.title).toBe('billing-service');
    expect(payload.visibility).toBe('public');
  });
});

describe('the initial BootstrapState', () => {
  it('parks on Name with a single-entry history and no way back', () => {
    const initial: BootstrapState = {
      step: BootstrapStep.Name,
      history: [BootstrapStep.Name],
    };

    expect(initial.step).toBe(BootstrapStep.Name);
    expect(initial.history).toEqual([BootstrapStep.Name]);
    expect(canRewind(initial)).toBe(false);
  });
});
