import { useCallback, useEffect, useMemo, useState, type ReactElement } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useNavigation, useProject } from '../../context';
import { useErrorBus } from '../../lib/errorBus';
import { formatError } from '../../lib/errors';
import { listProviderNamespaces, type ProviderNamespace } from '../../lib/createProjectWizard';
import { getAgentModels } from '../../lib/agentModels';
import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { reconcileEffort, type EffortLevel } from '../../lib/effortLevels';
import type { Machine, Provider } from '../../types';
import {
  BootstrapStep,
  STEP_ORDER,
  type BootstrapOutcome,
  type BootstrapState,
  type CreateProjectStepPayload,
} from '../../types';

import { WizardShell } from './WizardShell';
import { NameStep } from './NameStep';
import { ProviderStep } from './ProviderStep';
import { GroupStep } from './GroupStep';
import { MachineStep } from './MachineStep';
import { AgentStep } from './AgentStep';
import { ModelStep, type ModelOption } from './ModelStep';
import { DescriptionStep } from './DescriptionStep';

// ── Domain constants ──────────────────────────────────────────────────

/** Maximum number of namespaced to display before falling back to a
 *  simple list — matches the spec's "simple list v1" decision
 *  (OQ-4 in the implementation spec). */
const NAMESPACE_FETCH_LIMIT = 100;

// ── Local per-step snapshot ────────────────────────────────────────────

/** All wizard-collected values, kept in React state. The wizard
 *  reducer layers these onto the `CreateProjectStepPayload::Commit`
 *  snapshot at submit time so the Description step can edit just
 *  its own fields without the orchestrator having to track which
 *  fields the user last touched. */
interface WizardDraft {
  name: string;
  providerId: string;
  providerKind: string;
  providerHost: string;
  namespaceId: string;
  namespaceKind: 'personal' | 'org' | 'group' | '';
  namespaceName: string;
  machineKind: 'local' | 'remote';
  machineId: string | null;
  keyPassphrase: string;
  agentKind: string;
  model: string;
  effort: EffortLevel | '';
  title: string;
  description: string;
  visibility: 'private' | 'public';
}

const EMPTY_DRAFT: WizardDraft = {
  name: '',
  providerId: '',
  providerKind: '',
  providerHost: '',
  namespaceId: '',
  namespaceKind: '',
  namespaceName: '',
  machineKind: 'local',
  machineId: null,
  keyPassphrase: '',
  agentKind: '',
  model: '',
  effort: '',
  title: '',
  description: '',
  visibility: 'private',
};

// ── Main component ────────────────────────────────────────────────────

/**
 * The Create-Project wizard — full-screen, progressive-disclosure
 * flow that walks the user through **exactly seven**
 * one-decision-per-screen steps and then auto-launches the
 * `wf-starter-standard` workflow against the freshly-created repo.
 *
 * Step order: Name → Provider → Group → Machine → Agent → Model →
 * Description (locked, see `BootstrapStep`).
 *
 * **Architecture**
 *
 * - This component is the **only** place that calls `invoke()`. Each
 *   step component is presentational and emits a typed
 *   `CreateProjectStepPayload` upward via `onSubmit`.
 * - The wizard owns the canonical `BootstrapState` (mirroring the
 *   Rust state machine). `goBack` is **always** derived from the
 *   state's `history` — never by subtracting one from an index
 *   into `STEP_ORDER` (the latter silently re-enters
 *   auto-progressed screens).
 * - State machine transitions are delegated to the Rust side via
 *   `submit_create_project_step` / `go_back_create_project`. The
 *   React state simply reflects whatever the Rust command returns.
 */
export function CreateProjectWizard(): ReactElement {
  const { navigate } = useNavigation();
  const { state: projState, dispatch: projDispatch } = useProject();
  const { reportError } = useErrorBus();
  const { agents: agentCatalog } = useAgentCatalog();
  const agentKinds = useMemo(() => agentCatalog.map((a) => a.kind), [agentCatalog]);

  // The BootstrapState always reflects what the Rust side returned
  // last. Initialising to a fresh "Name with single-entry history"
  // shape mirrors `BootstrapState::new` so the UI is consistent
  // even before the first `begin_create_project` IPC resolves.
  const [bootstrap, setBootstrap] = useState<BootstrapState>(() => ({
    step: BootstrapStep.Name,
    history: [BootstrapStep.Name],
  }));
  const [draft, setDraft] = useState<WizardDraft>(EMPTY_DRAFT);
  const [providers, setProviders] = useState<ReadonlyArray<Provider>>([]);
  const [machines, setMachines] = useState<ReadonlyArray<Machine>>([]);
  const [namespaces, setNamespaces] = useState<ReadonlyArray<ProviderNamespace>>([]);
  const [namespacesLoading, setNamespacesLoading] = useState(false);
  const [models, setModels] = useState<ReadonlyArray<ModelOption>>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [committing, setCommitting] = useState(false);
  const [commitError, setCommitError] = useState<string | null>(null);

  // ── Begin a wizard session on mount ──────────────────────────────────
  //
  // The Rust command returns a fresh `BootstrapState` parked on
  // `Name` with a single-entry history. We use that as the source
  // of truth from then on; the React seed above only exists so the
  // UI renders something before the IPC resolves.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const initial = await invoke<BootstrapState>('begin_create_project');
        if (cancelled) return;
        setBootstrap(initial);
      } catch (err) {
        if (cancelled) return;
        reportError(err, { kind: 'internal' });
      }
    })();
    return () => { cancelled = true; };
  }, [reportError]);

  // ── Provider / machine lists (read from global store + RPC) ──────────
  useEffect(() => {
    setProviders(projState.providers);
  }, [projState.providers]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await invoke<Machine[]>('get_machines');
        if (cancelled) return;
        setMachines(list ?? []);
      } catch (err) {
        if (cancelled) return;
        reportError(err, { kind: 'internal' });
      }
    })();
    return () => { cancelled = true; };
  }, [reportError]);

  // ── Namespace fetch (when provider changes) ──────────────────────────
  useEffect(() => {
    let cancelled = false;
    if (!draft.providerId) {
      setNamespaces([]);
      return;
    }
    setNamespacesLoading(true);
    listProviderNamespaces(draft.providerId)
      .then((list) => {
        if (cancelled) return;
        const trimmed = list.slice(0, NAMESPACE_FETCH_LIMIT);
        setNamespaces(trimmed);
        const personal = trimmed.find((n) => n.kind === 'personal');
        setDraft((d) => ({
          ...d,
          namespaceId: personal?.id ?? trimmed[0]?.id ?? '',
          namespaceKind: (personal?.kind ?? trimmed[0]?.kind ?? '') as WizardDraft['namespaceKind'],
          namespaceName: personal?.name ?? trimmed[0]?.name ?? '',
        }));
      })
      .catch((err) => {
        if (cancelled) return;
        reportError(err, { kind: 'provider' });
        setNamespaces([]);
      })
      .finally(() => { if (!cancelled) setNamespacesLoading(false); });
    return () => { cancelled = true; };
  }, [draft.providerId, reportError]);

  // ── Model probe (machine + agent scoped) ─────────────────────────────
  //
  // Disabled until BOTH a machine AND an agent are picked — the
  // probe target depends on the machine, and the model set depends
  // on the agent, so neither field can be resolved without both.
  const probeEnabled =
    Boolean(draft.agentKind) &&
    (draft.machineKind === 'local' || Boolean(draft.machineId));
  const probeMachineId = draft.machineKind === 'remote'
    ? draft.machineId ?? ''
    : (draft.machineId || 'local');

  useEffect(() => {
    let cancelled = false;
    if (!probeEnabled || !probeMachineId || !draft.agentKind) {
      setModels([]);
      return;
    }
    setModelsLoading(true);
    getAgentModels(probeMachineId, draft.agentKind)
      .then((list) => {
        if (cancelled) return;
        setModels(list.map((m) => ({ value: m.value, name: m.name })));
      })
      .catch((err) => {
        if (cancelled) return;
        reportError(err, { kind: 'agent' });
        setModels([]);
      })
      .finally(() => { if (!cancelled) setModelsLoading(false); });
    return () => { cancelled = true; };
  }, [probeEnabled, probeMachineId, draft.agentKind, reportError]);

  // ── Submit helpers ──────────────────────────────────────────────────
  const submitStep = useCallback(async (payload: CreateProjectStepPayload) => {
    try {
      const outcome = await invoke<BootstrapOutcome>('submit_create_project_step', {
        state: bootstrap,
        payload,
      });
      if (outcome.kind === 'continue') {
        setBootstrap(outcome.state);
      } else {
        // `Launched` — feature is up; surface it on the global store
        // and navigate to its detail view.
        const launched = outcome.feature;
        projDispatch({
          type: 'ADD_PROJECT',
          project: {
            id: launched.project_id,
            name: draft.title || draft.name,
            status: 'idle',
            repos: 1,
            nodes: 0,
            spend: 0,
            tokens: 0,
            compute_type: draft.machineKind,
            remote_host: draft.machineKind === 'remote' ? (draft.machineId ?? null) : null,
          },
        });
        navigate({
          kind: 'detail',
          featureId: launched.feature_id,
          featureTitle: launched.feature_title,
        });
      }
    } catch (err) {
      reportError(err, { kind: 'validation' });
    }
  }, [bootstrap, draft, navigate, projDispatch, reportError]);

  // ── Per-step submit handlers (pure pass-through to `submitStep`) ────
  const onNameSubmit = useCallback(
    (payload: Extract<CreateProjectStepPayload, { step: 'name' }>) => {
      setDraft((d) => ({ ...d, name: payload.value }));
    },
    [],
  );
  const onProviderSubmit = useCallback(
    (payload: Extract<CreateProjectStepPayload, { step: 'provider' }>) => {
      const provider = providers.find((p) => p.id === payload.provider_id);
      setDraft((d) => ({
        ...d,
        providerId: payload.provider_id,
        providerKind: payload.kind,
        providerHost: provider?.host ?? '',
      }));
    },
    [providers],
  );
  const onGroupSubmit = useCallback(
    (payload: Extract<CreateProjectStepPayload, { step: 'group' }>) => {
      setDraft((d) => ({
        ...d,
        namespaceId: payload.namespace_id,
        namespaceKind: payload.kind as WizardDraft['namespaceKind'],
        namespaceName: payload.name,
      }));
    },
    [],
  );
  const onMachineSubmit = useCallback(
    (payload: Extract<CreateProjectStepPayload, { step: 'machine' }>) => {
      setDraft((d) => ({
        ...d,
        machineKind: payload.kind,
        machineId: payload.machine_id,
      }));
    },
    [],
  );
  const onAgentSubmit = useCallback(
    (payload: Extract<CreateProjectStepPayload, { step: 'agent' }>) => {
      // Clamp any effort already picked to what the (re)chosen agent accepts,
      // so going back to change the harness can't leave a stale, unrunnable
      // level on the draft.
      setDraft((d) => ({
        ...d,
        agentKind: payload.kind,
        effort: reconcileEffort(d.effort, effortLevelsFor(agentCatalog, payload.kind)),
      }));
    },
    [agentCatalog],
  );
  const onModelSubmit = useCallback(
    (payload: Extract<CreateProjectStepPayload, { step: 'model' }>) => {
      setDraft((d) => ({ ...d, model: payload.model, effort: payload.effort ?? '' }));
    },
    [],
  );
  const onCommitPatch = useCallback(
    (payload: Extract<CreateProjectStepPayload, { step: 'commit' }>) => {
      setDraft((d) => ({
        ...d,
        // Only Description-step-owned fields arrive here; the
        // orchestrator fills in the rest at submit time.
        title: payload.title,
        description: payload.description,
        visibility: payload.visibility,
      }));
    },
    [],
  );

  // ── Next button handler ─────────────────────────────────────────────
  const onNext = useCallback(async () => {
    switch (bootstrap.step) {
      case BootstrapStep.Name:
        if (draft.name.trim().length < 2) return;
        await submitStep({ step: 'name', value: draft.name.trim() });
        return;
      case BootstrapStep.Provider:
        if (!draft.providerId) return;
        await submitStep({
          step: 'provider',
          provider_id: draft.providerId,
          kind: draft.providerKind,
        });
        return;
      case BootstrapStep.Group:
        if (!draft.namespaceId) return;
        await submitStep({
          step: 'group',
          namespace_id: draft.namespaceId,
          kind: draft.namespaceKind || 'org',
          name: draft.namespaceName,
        });
        return;
      case BootstrapStep.Machine:
        if (draft.machineKind === 'remote' && !draft.machineId) return;
        // Write passphrase to keyring BEFORE the bootstrap clone
        // runs (mirrors NewProjectView.tsx:164-173). The Commit
        // payload will follow.
        if (draft.machineKind === 'remote' && draft.machineId && draft.keyPassphrase.trim().length > 0) {
          try {
            await invoke('set_machine_secret', {
              machineId: draft.machineId,
              secret: draft.keyPassphrase,
            });
            setDraft((d) => ({ ...d, keyPassphrase: '' }));
          } catch (err) {
            reportError(err, { kind: 'transport' });
            return;
          }
        }
        await submitStep({
          step: 'machine',
          kind: draft.machineKind,
          machine_id: draft.machineId,
        });
        return;
      case BootstrapStep.Agent:
        if (!draft.agentKind) return;
        await submitStep({ step: 'agent', kind: draft.agentKind });
        return;
      case BootstrapStep.Model:
        if (!draft.model.trim()) return;
        await submitStep({ step: 'model', model: draft.model.trim(), effort: draft.effort || null });
        return;
      case BootstrapStep.Description: {
        // Final step — emit the Commit payload with the full
        // snapshot. The Rust side runs the entire
        // create-remote-repo → persist-project → bootstrap →
        // save-settings → start-feature chain.
        if (draft.title.trim().length < 1 || draft.description.trim().length < 8) return;
        setCommitting(true);
        setCommitError(null);
        try {
          await submitStep({
            step: 'commit',
            title: draft.title.trim() || draft.name.trim(),
            description: draft.description.trim(),
            visibility: draft.visibility,
            name: draft.name.trim(),
            provider_id: draft.providerId,
            provider_kind: draft.providerKind,
            provider_host: draft.providerHost,
            namespace_id: draft.namespaceId,
            namespace_kind: draft.namespaceKind || 'org',
            namespace_name: draft.namespaceName,
            machine_kind: draft.machineKind,
            machine_id: draft.machineId,
            agent_kind: draft.agentKind,
            model: draft.model.trim(),
            effort: draft.effort || null,
          });
        } catch (err) {
          setCommitError(formatError(err));
        } finally {
          setCommitting(false);
        }
        return;
      }
    }
  }, [bootstrap.step, draft, submitStep, reportError]);

  // ── Back button handler ─────────────────────────────────────────────
  //
  // CRITICAL: must always call `go_back_create_project` so the
  // history-pop logic lives on the Rust side. Decrementing an
  // index into STEP_ORDER here silently re-enters auto-progressed
  // screens (e.g. when only one provider is configured, the wizard
  // auto-skips Provider — an index-based goBack would then jump
  // from Machine straight to Name, bypassing the auto-progressed
  // Provider entry in history and discarding the user's choice).
  const onBack = useCallback(async () => {
    if (!canRewind(bootstrap)) return;
    try {
      const rewound = await invoke<BootstrapState>('go_back_create_project', { state: bootstrap });
      setBootstrap(rewound);
    } catch (err) {
      reportError(err, { kind: 'internal' });
    }
  }, [bootstrap, reportError]);

  // ── Per-step gating ─────────────────────────────────────────────────
  // Mirrors the per-step Validation arms in the Rust
  // `submit_create_project_step` command.
  const gateReason = useMemo<string>(() => {
    switch (bootstrap.step) {
      case BootstrapStep.Name:
        return draft.name.trim().length < 2 ? 'Type a project name' : '';
      case BootstrapStep.Provider:
        return !draft.providerId ? 'Pick a provider' : '';
      case BootstrapStep.Group:
        return !draft.namespaceId ? 'Pick a namespace' : '';
      case BootstrapStep.Machine:
        return draft.machineKind === 'remote' && !draft.machineId
          ? 'Select a remote machine'
          : '';
      case BootstrapStep.Agent:
        return !draft.agentKind ? 'Pick a coding agent' : '';
      case BootstrapStep.Model:
        return !draft.model.trim() ? 'Pick or type a model' : '';
      case BootstrapStep.Description:
        if (draft.title.trim().length < 1) return 'Feature title required';
        if (draft.description.trim().length < 8) return 'Describe the feature in a sentence or two';
        if (committing) return 'Launching…';
        return commitError ?? '';
    }
  }, [bootstrap.step, draft, committing, commitError]);

  const canProceed = gateReason === '';
  const isFinal = bootstrap.step === BootstrapStep.Description;

  // ── Render ──────────────────────────────────────────────────────────
  return (
    <WizardShell
      step={bootstrap.step}
      history={bootstrap.history}
      canProceed={canProceed}
      reason={gateReason}
      isFinal={isFinal}
      nextLabel={isFinal ? (committing ? 'Launching…' : 'Create project') : undefined}
      onBack={onBack}
      onNext={() => void onNext()}
    >
      {bootstrap.step === BootstrapStep.Name && (
        <NameStep value={draft.name} onSubmit={onNameSubmit} />
      )}

      {bootstrap.step === BootstrapStep.Provider && (
        <ProviderStep
          providers={providers}
          value={draft.providerId}
          onSubmit={onProviderSubmit}
        />
      )}

      {bootstrap.step === BootstrapStep.Group && (
        <GroupStep
          namespaces={namespaces}
          loading={namespacesLoading}
          value={draft.namespaceId}
          onSubmit={onGroupSubmit}
        />
      )}

      {bootstrap.step === BootstrapStep.Machine && (
        <MachineStep
          machines={machines}
          kind={draft.machineKind}
          machineId={draft.machineId ?? ''}
          keyPassphrase={draft.keyPassphrase}
          onSubmit={onMachineSubmit}
          onPassphraseChange={(v) => setDraft((d) => ({ ...d, keyPassphrase: v }))}
        />
      )}

      {bootstrap.step === BootstrapStep.Agent && (
        <AgentStep agentKinds={agentKinds} value={draft.agentKind} onSubmit={onAgentSubmit} />
      )}

      {bootstrap.step === BootstrapStep.Model && (
        <ModelStep
          enabled={probeEnabled}
          loading={modelsLoading}
          models={models}
          value={draft.model}
          effort={draft.effort}
          effortLevels={effortLevelsFor(agentCatalog, draft.agentKind)}
          onSubmit={onModelSubmit}
        />
      )}

      {bootstrap.step === BootstrapStep.Description && (
        <DescriptionStep
          description={draft.description}
          title={draft.title || draft.name}
          visibility={draft.visibility}
          onSubmit={onCommitPatch}
        />
      )}
    </WizardShell>
  );
}

// ── Exported pure helpers (unit-test surface) ──────────────────────────

/** True iff the wizard's history allows a backward step. Mirrors
 *  Rust `BootstrapState::can_go_back` and the `disabled` attribute
 *  the shell renders on the Back button. */
export function canRewind(state: BootstrapState): boolean {
  return state.history.length > 1;
}

/** Build the final `CreateProjectStepPayload::Commit` payload from a
 *  wizard draft. Centralised so unit tests can assert the exact
 *  wire shape without mounting React. */
export function buildCommitPayload(draft: WizardDraft): Extract<CreateProjectStepPayload, { step: 'commit' }> {
  return {
    step: 'commit',
    title: (draft.title.trim() || draft.name.trim()),
    description: draft.description.trim(),
    visibility: draft.visibility,
    name: draft.name.trim(),
    provider_id: draft.providerId,
    provider_kind: draft.providerKind,
    provider_host: draft.providerHost,
    namespace_id: draft.namespaceId,
    namespace_kind: draft.namespaceKind || 'org',
    namespace_name: draft.namespaceName,
    machine_kind: draft.machineKind,
    machine_id: draft.machineId,
    agent_kind: draft.agentKind,
    model: draft.model.trim(),
    effort: draft.effort || null,
  };
}

/** Single source of truth for the seven step slugs — also
 *  re-exported here so the unit tests don't have to depend on
 *  `STEP_ORDER` from types.ts. */
export { STEP_ORDER };

export default CreateProjectWizard;