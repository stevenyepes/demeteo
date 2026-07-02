import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getAgentModels } from '../../lib/agentModels';
import { listProviderNamespaces, type ProviderNamespace } from '../../lib/createProjectWizard';
import { useErrorBus } from '../../lib/errorBus';
import { useProject } from '../../context';
import type { Machine, Provider, WorkflowSummary } from '../../types';

const WORKFLOW_ID_STARTER = 'wf-starter-standard';

export interface WizardFormState {
  projectName: string;
  providerId: string;
  namespaceId: string;
  repoSlug: string;
  repoPrivate: boolean;
  machineKind: 'local' | 'remote';
  machineId: string;
  keyPassphrase: string;
  agentKind: string;
  model: string;
  defaultBranch: string;
  branchPrefix: string;
  testCommand: string;
  prTemplate: string;
  conflictPolicy: string;
  featureLifecycle: string;
  description: string;
  workflowId: string;
}

export interface WizardFormSetters {
  setProjectName: (v: string) => void;
  setProviderId: (v: string) => void;
  setNamespaceId: (v: string) => void;
  setRepoSlug: (v: string) => void;
  setRepoPrivate: (v: boolean) => void;
  setMachineKind: (v: 'local' | 'remote') => void;
  setMachineId: (v: string) => void;
  setKeyPassphrase: (v: string) => void;
  setAgentKind: (v: string) => void;
  setModel: (v: string) => void;
  setDefaultBranch: (v: string) => void;
  setBranchPrefix: (v: string) => void;
  setTestCommand: (v: string) => void;
  setPrTemplate: (v: string) => void;
  setConflictPolicy: (v: string) => void;
  setFeatureLifecycle: (v: string) => void;
  setDescription: (v: string) => void;
  setWorkflowId: (v: string) => void;
}

export interface WizardFormApi extends WizardFormState, WizardFormSetters {
  /** Connected provider instances (read from the global project store). */
  providers: ReadonlyArray<Provider>;
  namespaces: ProviderNamespace[];
  namespacesLoading: boolean;
  machines: Machine[];
  workflows: WorkflowSummary[];
  models: { value: string; name: string }[];
  modelsLoading: boolean;
  /** Resolved projectId once the bootstrap pipeline finishes. */
  projectId: string | null;
  setProjectId: (id: string | null) => void;
  /** Latest strategy snapshot — exposed so the wizard can hydrate the
   *  review form with `test_command` / `pr_template` defaults. */
  applyStrategyToForm: (strategy: {
    default_branch: string;
    branch_prefix: string;
    test_command?: string | null;
    pr_template?: string | null;
  }) => void;
}

/**
 * All the form state + side effects for the Create-From-Zero wizard,
 * extracted so the main component stays focused on rendering and
 * step transitions. Returns a flat shape that mirrors the existing
 * per-field useState calls so the JSX can be lifted with minimal
 * diff noise.
 */
export function useCreateZeroWizardForm(): WizardFormApi {
  const { reportError } = useErrorBus();
  const { state: proj } = useProject();
  const { providers } = proj;
  const [projectName, setProjectName] = useState('');
  const [providerId, setProviderId] = useState('');
  const [namespaceId, setNamespaceId] = useState('');
  const [repoSlug, setRepoSlug] = useState('');
  const [repoPrivate, setRepoPrivate] = useState(true);
  const [namespaces, setNamespaces] = useState<ProviderNamespace[]>([]);
  const [namespacesLoading, setNamespacesLoading] = useState(false);
  const [machineKind, setMachineKind] = useState<'local' | 'remote'>('local');
  const [machines, setMachines] = useState<Machine[]>([]);
  const [machineId, setMachineId] = useState('');
  const [keyPassphrase, setKeyPassphrase] = useState('');
  const [agentKind, setAgentKind] = useState('');
  const [model, setModel] = useState('');
  const [models, setModels] = useState<{ value: string; name: string }[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [defaultBranch, setDefaultBranch] = useState('main');
  const [branchPrefix, setBranchPrefix] = useState('demeteo/features/');
  const [testCommand, setTestCommand] = useState('');
  const [prTemplate, setPrTemplate] = useState('');
  const [conflictPolicy, setConflictPolicy] = useState('always_gate');
  const [featureLifecycle, setFeatureLifecycle] = useState('archive');
  const [description, setDescription] = useState('');
  const [workflows, setWorkflows] = useState<WorkflowSummary[]>([]);
  const [workflowId, setWorkflowId] = useState('');

  // Mount-time data fetches: machines + workflows. Both lists are
  // needed by step UIs that load before the user reaches them, so we
  // pre-fetch to hide latency behind typing. Each fetch owns its own
  // cancellation flag so a slow second fetch can't suppress the
  // first's state update.
  useEffect(() => {
    let machinesCancelled = false;
    let workflowsCancelled = false;
    (async () => {
      try {
        const list = (await invoke<Machine[]>('get_machines')) ?? [];
        if (!machinesCancelled) setMachines(list);
      } catch (err) { reportError(err, { kind: 'internal' }); }
    })();
    (async () => {
      try {
        const list = (await invoke<WorkflowSummary[]>('workflow_list')) ?? [];
        if (workflowsCancelled) return;
        setWorkflows(list);
        const starter = list.find((w) => w.id === WORKFLOW_ID_STARTER);
        setWorkflowId(starter?.id ?? list[0]?.id ?? '');
      } catch (err) { reportError(err, { kind: 'internal' }); }
    })();
    return () => { machinesCancelled = true; workflowsCancelled = true; };
  }, [reportError]);

  // Whenever the provider changes, fetch its namespaces via the
  // sub-1 wrapper and auto-pick the personal namespace.
  useEffect(() => {
    let cancelled = false;
    if (!providerId) { setNamespaces([]); setNamespaceId(''); return; }
    setNamespacesLoading(true);
    listProviderNamespaces(providerId)
      .then((list) => {
        if (cancelled) return;
        setNamespaces(list);
        const personal = list.find((n) => n.kind === 'personal');
        setNamespaceId(personal?.id ?? list[0]?.id ?? '');
      })
      .catch((err) => {
        if (cancelled) return;
        reportError(err, { kind: 'provider' });
        setNamespaces([]);
        setNamespaceId('');
      })
      .finally(() => { if (!cancelled) setNamespacesLoading(false); });
    return () => { cancelled = true; };
  }, [providerId, reportError]);

  // Probe the model list whenever machine + agent are both set. The
  // probe is machine-scoped — local and remote machines can advertise
  // different model sets.
  useEffect(() => {
    let cancelled = false;
    const probeMachineId = machineKind === 'remote' ? machineId : machineId || 'local';
    if (!probeMachineId || !agentKind) { setModels([]); return; }
    setModelsLoading(true);
    getAgentModels(probeMachineId, agentKind)
      .then((list) => {
        if (!cancelled) setModels(list.map((m) => ({ value: m.value, name: m.name })));
      })
      .catch((err) => {
        if (!cancelled) {
          reportError(err, { kind: 'agent' });
          setModels([]);
        }
      })
      .finally(() => { if (!cancelled) setModelsLoading(false); });
    return () => { cancelled = true; };
  }, [machineKind, machineId, agentKind, reportError]);

  // Reset passphrase if the user toggles machine kind — otherwise the
  // next submit would write the old machine's passphrase somewhere
  // it doesn't belong.
  useEffect(() => { setKeyPassphrase(''); }, [machineKind]);

  const applyStrategyToForm = useCallback((strategy: {
    default_branch: string;
    branch_prefix: string;
    test_command?: string | null;
    pr_template?: string | null;
  }) => {
    setDefaultBranch(strategy.default_branch);
    setBranchPrefix(strategy.branch_prefix);
    setTestCommand(strategy.test_command ?? '');
    setPrTemplate(strategy.pr_template ?? '');
  }, []);

  return useMemo<WizardFormApi>(() => ({
    projectName, providerId, namespaceId, repoSlug, repoPrivate,
    machineKind, machineId, keyPassphrase, agentKind, model,
    defaultBranch, branchPrefix, testCommand, prTemplate,
    conflictPolicy, featureLifecycle, description, workflowId,
    providers, namespaces, namespacesLoading, machines, workflows,
    models, modelsLoading, projectId,
    setProjectName, setProviderId, setNamespaceId, setRepoSlug, setRepoPrivate,
    setMachineKind, setMachineId, setKeyPassphrase, setAgentKind, setModel,
    setDefaultBranch, setBranchPrefix, setTestCommand, setPrTemplate,
    setConflictPolicy, setFeatureLifecycle, setDescription, setWorkflowId,
    setProjectId, applyStrategyToForm,
  }), [
    projectName, providerId, namespaceId, repoSlug, repoPrivate,
    machineKind, machineId, keyPassphrase, agentKind, model,
    defaultBranch, branchPrefix, testCommand, prTemplate,
    conflictPolicy, featureLifecycle, description, workflowId,
    providers, namespaces, namespacesLoading, machines, workflows,
    models, modelsLoading, projectId,
    applyStrategyToForm,
  ]);
}
