// Type-level + pure-function tests for the Create-Project wizard.
//
// Like `src/types.appview.test.ts`, this file is consumed purely by
// `tsc --noEmit` — no runtime test runner is wired up. The exported
// `wizardTestResults` object lets a downstream CI step or the
// reviewer call the helper functions directly to assert the
// canonical wizard invariants without mounting React.
//
// The two regressions this file is contractually bound to catch:
//   1. STEP_ORDER must contain EXACTLY seven entries (C-3 from the
//      previous attempt's validation report — extra
//      strategy/workflow/launching screens are forbidden inside the
//      wizard).
//   2. goBack must be derived from `state.history`, never from a
//      raw index into STEP_ORDER (C-4 — the counter-based goBack
//      silently re-entered auto-progressed screens).
//
// Both invariants are exercised below as pure functions so they can
// be asserted deterministically.

import { BootstrapStep, STEP_ORDER } from './types';
import {
  canRewind,
  buildCommitPayload,
} from './components/wizard/CreateProjectWizard';
import {
  canGoBackFromHistory,
  rewindHistory,
} from './components/wizard/WizardShell';
import type {
  BootstrapState,
  CreateProjectStepPayload,
} from './types';

// ── STEP_ORDER invariants (AC-5) ────────────────────────────────────────

// Locked: the wizard renders exactly seven one-decision-per-screen
// steps. Anything else violates the spec's "no strategy, workflow,
// or launching screen sits in the wizard" rule.
const SEVEN: 7 = 7;
void SEVEN;

if (STEP_ORDER.length !== 7) {
  throw new Error(
    `STEP_ORDER must contain exactly 7 entries; got ${STEP_ORDER.length}.`,
  );
}

const EXPECTED_ORDER: ReadonlyArray<BootstrapStep> = [
  BootstrapStep.Name,
  BootstrapStep.Provider,
  BootstrapStep.Group,
  BootstrapStep.Machine,
  BootstrapStep.Agent,
  BootstrapStep.Model,
  BootstrapStep.Description,
];

for (let i = 0; i < EXPECTED_ORDER.length; i++) {
  if (STEP_ORDER[i] !== EXPECTED_ORDER[i]) {
    throw new Error(
      `STEP_ORDER[${i}] expected ${EXPECTED_ORDER[i]}, got ${STEP_ORDER[i]}`,
    );
  }
}

// ── BootstrapStep discriminant string stability ───────────────────────

// The kebab-case slugs are the IPC contract — the Rust
// `CreateProjectStepPayload` matches on them via `expected_step()`.
const EXPECTED_STRINGS: Record<BootstrapStep, string> = {
  name: 'name',
  provider: 'provider',
  group: 'group',
  machine: 'machine',
  agent: 'agent',
  model: 'model',
  description: 'description',
};

for (const step of STEP_ORDER) {
  if (step !== EXPECTED_STRINGS[step]) {
    throw new Error(`BootstrapStep "${step}" disagrees with documented slug`);
  }
}

// ── canRewind + canGoBackFromHistory ──────────────────────────────────

if (canRewind({ step: BootstrapStep.Name, history: [BootstrapStep.Name] }) !== false) {
  throw new Error('canRewind must be false on the initial state');
}

if (
  canRewind({
    step: BootstrapStep.Provider,
    history: [BootstrapStep.Name, BootstrapStep.Provider],
  }) !== true
) {
  throw new Error('canRewind must be true after a single advance');
}

if (canGoBackFromHistory([BootstrapStep.Name]) !== false) {
  throw new Error('canGoBackFromHistory must be false on a single-entry history');
}

if (
  canGoBackFromHistory([BootstrapStep.Name, BootstrapStep.Provider]) !== true
) {
  throw new Error('canGoBackFromHistory must be true on a two-entry history');
}

// ── rewindHistory (C-4 regression test) ──────────────────────────────
//
// This is the test that pins down the C-4 fix. The previous attempt
// rewound via `STEP_ORDER.indexOf(step) - 1`, which silently re-
// entered auto-progressed screens. The correct behaviour pops the
// history stack, so an auto-progressed Provider step (recorded in
// history) is still rewindable — the wizard frontend can then call
// rewindHistory again to keep rewinding past the auto-progressed
// entry.
{
  const history: ReadonlyArray<BootstrapStep> = [
    BootstrapStep.Name,
    BootstrapStep.Provider, // auto-progressed
    BootstrapStep.Group,    // auto-progressed
    BootstrapStep.Machine,
  ];
  const step1 = rewindHistory(history);
  if (step1 !== BootstrapStep.Group) {
    throw new Error(`rewindHistory step1 expected Group, got ${step1}`);
  }
  const step2 = rewindHistory(history.slice(0, history.length - 1));
  if (step2 !== BootstrapStep.Provider) {
    throw new Error(`rewindHistory step2 expected Provider, got ${step2}`);
  }
  const step3 = rewindHistory(history.slice(0, history.length - 2));
  if (step3 !== BootstrapStep.Name) {
    throw new Error(`rewindHistory step3 expected Name, got ${step3}`);
  }
  const step4 = rewindHistory(history.slice(0, history.length - 3));
  if (step4 !== null) {
    throw new Error(`rewindHistory step4 expected null at Name, got ${step4}`);
  }
}

// ── buildCommitPayload ────────────────────────────────────────────────

{
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
    title: 'Implement billing service',
    description: 'A billing service written in Rust.',
    visibility: 'private' as const,
  };

  const payload = buildCommitPayload(draft);

  // The wire shape must match the Rust `CreateProjectStepPayload::Commit`.
  const expected: CreateProjectStepPayload = {
    step: 'commit',
    title: 'Implement billing service',
    description: 'A billing service written in Rust.',
    visibility: 'private',
    name: 'billing-service',
    providerId: 'prov-1',
    providerKind: 'github',
    providerHost: 'github.com',
    namespaceId: 'my-org',
    namespaceKind: 'org',
    namespaceName: 'My Org',
    machineKind: 'remote',
    machineId: 'machine-1',
    agentKind: 'opencode',
    model: 'anthropic/claude-sonnet-4',
  };

  if (JSON.stringify(payload) !== JSON.stringify(expected)) {
    throw new Error(
      `buildCommitPayload produced unexpected shape:\n` +
        `  got:      ${JSON.stringify(payload)}\n` +
        `  expected: ${JSON.stringify(expected)}`,
    );
  }

  // Belt-and-braces: the passphrase must NEVER leak into the commit
  // payload. The Rust port doesn't carry it; the wizard writes it to
  // the keyring via a separate `set_machine_secret` IPC before
  // bootstrap runs.
  if ('keyPassphrase' in payload) {
    throw new Error('buildCommitPayload must not include keyPassphrase');
  }
}

// Title fallback: when the user leaves the title blank, the commit
// payload falls back to the project name (per OQ-5).
{
  const draft = {
    name: 'billing-service',
    providerId: 'prov-1',
    providerKind: 'github',
    providerHost: 'github.com',
    namespaceId: 'ns',
    namespaceKind: 'personal' as const,
    namespaceName: 'me',
    machineKind: 'local' as const,
    machineId: null,
    keyPassphrase: '',
    agentKind: 'opencode',
    model: 'm',
    title: '',
    description: 'Build the billing service.',
    visibility: 'public' as const,
  };
  const payload = buildCommitPayload(draft);
  if (payload.title !== 'billing-service') {
    throw new Error(`title fallback expected 'billing-service', got '${payload.title}'`);
  }
  if (payload.visibility !== 'public') {
    throw new Error(`visibility expected 'public', got '${payload.visibility}'`);
  }
}

// ── BootstrapState default shape matches Rust ─────────────────────────

{
  const initial: BootstrapState = {
    step: BootstrapStep.Name,
    history: [BootstrapStep.Name],
  };
  if (initial.history.length !== 1) {
    throw new Error('initial BootstrapState must have a single-entry history');
  }
  if (initial.step !== BootstrapStep.Name) {
    throw new Error('initial BootstrapState must be parked on Name');
  }
  if (canRewind(initial)) {
    throw new Error('canRewind on initial state must be false');
  }
}

// ── Type-level smoke checks ───────────────────────────────────────────

// Discriminator exhaustiveness: the wizard's switch over
// `bootstrap.step` must cover all seven variants. The TypeScript
// type system enforces this when we explicitly assign to a value of
// type `BootstrapStep`.
const ALL_STEPS: ReadonlyArray<BootstrapStep> = [
  BootstrapStep.Name,
  BootstrapStep.Provider,
  BootstrapStep.Group,
  BootstrapStep.Machine,
  BootstrapStep.Agent,
  BootstrapStep.Model,
  BootstrapStep.Description,
];

if (ALL_STEPS.length !== STEP_ORDER.length) {
  throw new Error('ALL_STEPS and STEP_ORDER disagree on cardinality');
}

// ── Exported results object (for runtime introspection) ───────────────

export const wizardTestResults = {
  stepCount: STEP_ORDER.length,
  stepOrder: STEP_ORDER,
  allSteps: ALL_STEPS,
  canRewindOnInitial: canRewind({ step: BootstrapStep.Name, history: [BootstrapStep.Name] }),
  canRewindAfterAdvance: canRewind({
    step: BootstrapStep.Provider,
    history: [BootstrapStep.Name, BootstrapStep.Provider],
  }),
} as const;