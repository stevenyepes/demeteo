import { createContext, useContext, useState, useEffect } from 'react';
import type { ConfigOptionValue, EffortLevel, ProjectMemoryEntry, StepConfig, Machine, Project } from '../../types';
import { getAgentModels } from '../../lib/agentModels';
import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { reconcileDefaultWorkflow } from '../../lib/workflowDefault';
import { DEFAULT_EFFORT, reconcileEffort } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import { useErrorBus } from '../../lib/errorBus';
import {
  checkReposDirty,
  deleteProject,
  deleteProjectMemory,
  getProposedStrategy,
  getRepositoriesForProject,
  getWorkflowOverrides,
  getWorkspaceHealth,
  listProjectMemory,
  probeProjectCommands,
  saveProjectSettings,
  setWorkflowOverride,
  updateProject,
  upsertProjectMemory,
  type CommandProbeReport,
  type RepoDirtyStatus,
  type RepoHealthStatus,
} from '../../lib/project';
import {
  getAgentConfigs,
  listMachines,
  setAgentConfigs as writeAgentConfigs,
  testMachineConnection,
  type AgentConfigView,
} from '../../lib/machines';
import { fetchProviderRepos } from '../../lib/providers';
import { bootstrapProject } from '../../lib/createProjectWizard';
import { listWorkflows } from '../../lib/workflows';
import { useNavigation, useProject } from '../../context';

export interface AvailableRepo { path: string; providerId: string; }
export type { RepoDirtyStatus, RepoHealthStatus, WorktreeInfo } from '../../lib/project';
export type { AgentConfigView } from '../../lib/machines';

/** How long the panel waits after a keystroke before asking the machine. A
 *  probe per character would be a `command -v` per character, on a machine
 *  that may be an SSH hop away. */
const PROBE_DEBOUNCE_MS = 400;

export const WF_LEVEL = '';
export const ovKey = (workflowId: string, stepId: string) => `${workflowId}::${stepId}`;

/** One row of the project's workflow/step override table. Every field is
 *  independently `null` = "inherit that one field". */
export interface OverrideRowValue {
  agent_kind: string | null;
  model: string | null;
  effort: EffortLevel | null;
}

/** A row that overrides nothing. Sending this clears the row server-side (the
 *  repo deletes an all-`null` override). */
export const EMPTY_ROW: OverrideRowValue = { agent_kind: null, model: null, effort: null };

/** True when the row pins at least one field. */
export const isOverrideActive = (o: OverrideRowValue | undefined): boolean =>
  Boolean(o?.agent_kind || o?.model || o?.effort);

interface SettingsCtx {
  // project context
  activeProject: Project;
  navigate: ReturnType<typeof useNavigation>['navigate'];
  // loading / status
  isLoading: boolean;
  activeTab: 'general' | 'strategy' | 'overrides' | 'memory';
  setActiveTab: (t: 'general' | 'strategy' | 'overrides' | 'memory') => void;
  status: 'idle' | 'saving' | 'success' | 'error';
  errorMsg: string;
  // memory
  memories: ProjectMemoryEntry[]; isMemoriesLoading: boolean;
  editingMemory: ProjectMemoryEntry | null; setEditingMemory: (v: ProjectMemoryEntry | null) => void;
  newMemKey: string; setNewMemKey: (v: string) => void;
  newMemVal: string; setNewMemVal: (v: string) => void;
  memError: string;
  // general
  projectName: string; setProjectName: (v: string) => void;
  computeType: string; setComputeType: (v: string) => void;
  remoteHost: string; setRemoteHost: (v: string) => void;
  machines: Machine[];
  isTestingConnection: boolean; connectionStatus: 'idle' | 'success' | 'error';
  selectedRepos: AvailableRepo[]; originalRepos: AvailableRepo[];
  isRepoModalOpen: boolean; setIsRepoModalOpen: (v: boolean) => void;
  repoSearch: string; setRepoSearch: (v: string) => void;
  availableRepos: AvailableRepo[]; isLoadingRepos: boolean;
  bootstrapStep: 'form' | 'bootstrapping' | 'strategy_proposal' | 'bootstrap_success' | 'error';
  setBootstrapStep: (v: 'form' | 'bootstrapping' | 'strategy_proposal' | 'bootstrap_success' | 'error') => void;
  bootstrapError: string;
  healthData: RepoHealthStatus[] | null; isLoadingHealth: boolean;
  healthExpanded: boolean; setHealthExpanded: (v: boolean) => void;
  showHealthPanel: boolean; healthError: string;
  // strategy
  defaultBranch: string; setDefaultBranch: (v: string) => void;
  branchPrefix: string; setBranchPrefix: (v: string) => void;
  testCommand: string; setTestCommand: (v: string) => void;
  buildCommand: string; setBuildCommand: (v: string) => void;
  coverageCommand: string; setCoverageCommand: (v: string) => void;
  conventionsFile: string; setConventionsFile: (v: string) => void;
  harnesses: { [key: string]: string }; setHarnesses: (v: { [key: string]: string }) => void;
  /** The harnesses that gate validation, in the order they run. Tier 2 of the
   *  engine's resolution chain; empty = fall through to `test_command`. */
  validationGates: string[]; setValidationGates: (v: string[]) => void;
  /** Latest per-command probe of the *project's* machine, or `null` when none
   *  has answered yet. An indicator only — nothing here gates a save. */
  commandProbe: CommandProbeReport | null;
  isProbingCommands: boolean;
  /** Why the probe could not answer (machine unreachable, none selected, …).
   *  Rendered beside the rows; never a reason to refuse a save. */
  probeError: string;
  refreshCommandProbe: () => void;
  prepareCommand: string; setPrepareCommand: (v: string) => void;
  prTemplate: string; setPrTemplate: (v: string) => void;
  conflictPolicy: string; setConflictPolicy: (v: string) => void;
  featureLifecycle: string; setFeatureLifecycle: (v: string) => void;
  defaultAgentKind: string; setDefaultAgentKind: (v: string) => void;
  defaultModel: string; setDefaultModel: (v: string) => void;
  /** Project-wide default reasoning effort. `''` = no project default, which
   *  resolves to the engine default (`high`) at run time. */
  defaultEffort: EffortLevel | ''; setDefaultEffort: (v: EffortLevel | '') => void;
  /** The workflow a new feature in this project starts on. `''` = not chosen,
   *  which persists as `null` — the launch modal falls back explicitly on it
   *  rather than taking whatever `workflow_list` returned first. */
  defaultWorkflowId: string; setDefaultWorkflowId: (v: string) => void;
  /** The id the stored default named before its workflow was deleted, kept so
   *  the picker can say *which* choice it dropped. `null` once the user picks
   *  again. */
  missingDefaultWorkflowId: string | null;
  defaultLoopIterations: string; setDefaultLoopIterations: (v: string) => void;
  defaultMaxBudgetUsd: string; setDefaultMaxBudgetUsd: (v: string) => void;
  extraWritablePaths: string[]; setExtraWritablePaths: (v: string[]) => void;
  newExtraPath: string; setNewExtraPath: (v: string) => void;
  availableModelsForDefault: ConfigOptionValue[]; isLoadingModelsForDefault: boolean;
  agentConfigs: AgentConfigView[]; setAgentConfigs: (v: AgentConfigView[]) => void;
  isRefreshingAgents: boolean;
  artifactSubdir: string; setArtifactSubdir: (v: string) => void;
  commitArtifacts: boolean; setCommitArtifacts: (v: boolean) => void;
  /** The command a reviewing step starts from, verbatim. `''` = the project
   *  names none, which persists as `null` and leaves the step to review in
   *  its own way. */
  reviewEntrypoint: string; setReviewEntrypoint: (v: string) => void;
  /** The harness/model/effort a merge-conflict resolution runs under. `''` on
   *  all three = no opinion, which persists as `null` and inherits the run and
   *  then the project defaults. */
  syncResolverAgentKind: string; setSyncResolverAgentKind: (v: string) => void;
  syncResolverModel: string; setSyncResolverModel: (v: string) => void;
  syncResolverEffort: EffortLevel | ''; setSyncResolverEffort: (v: EffortLevel | '') => void;
  availableModelsForSyncResolver: ConfigOptionValue[]; isLoadingModelsForSyncResolver: boolean;
  // warning modals
  dirtyWarningRepos: RepoDirtyStatus[];
  setDirtyWarningRepos: (v: RepoDirtyStatus[]) => void;
  pendingActionAfterConfirm: 'save' | 'delete' | null;
  setPendingActionAfterConfirm: (v: 'save' | 'delete' | null) => void;
  showDeleteConfirm: boolean; setShowDeleteConfirm: (v: boolean) => void;
  // overrides
  workflows: { id: string; name: string; description: string; steps: StepConfig[] }[];
  /** True once `workflow_list` has answered. Distinguishes "no workflows" from
   *  "not asked yet", which the default-workflow picker cannot conflate: an
   *  unanswered list would degrade every stored id to unset. */
  workflowsLoaded: boolean;
  workflowsError: string;
  overrides: Record<string, OverrideRowValue>;
  setOverrides: (v: Record<string, OverrideRowValue>) => void;
  isLoadingOverrides: boolean; overridesError: string;
  expandedWf: Record<string, boolean>; setExpandedWf: (v: Record<string, boolean>) => void;
  rowModels: Record<string, ConfigOptionValue[]>;
  rowModelsLoading: Record<string, boolean>;
  savedPulse: Record<string, boolean>;
  overrideAgentKinds: string[];
  overridesMachineId: string;
  // handlers
  handleSave: () => void;
  handleDeleteClick: () => void;
  proceedWithReBootstrap: () => void;
  proceedWithDelete: () => void;
  handleApproveStrategy: () => void;
  handleSaveMemory: (e: React.FormEvent) => void;
  handleDeleteMemory: (id: string) => void;
  handleEditMemoryClick: (entry: ProjectMemoryEntry) => void;
  handleCancelEdit: () => void;
  fetchAllReposFromProviders: () => void;
  toggleRepo: (repo: AvailableRepo) => void;
  handleTestConnection: () => void;
  fetchWorkspaceHealth: () => void;
  fetchAgentConfigs: (refresh?: boolean) => void;
  toggleWorkflowExpanded: (wf: { id: string; steps: StepConfig[] }) => void;
  handleAgentChange: (wfId: string, stepId: string, step: StepConfig | null, agentKind: string) => void;
  handleModelChange: (wfId: string, stepId: string, model: string) => void;
  handleEffortChange: (wfId: string, stepId: string, effort: EffortLevel | '') => void;
  handleClearRow: (wfId: string, stepId: string) => void;
  workflowOverrideCount: (wf: { id: string; steps: StepConfig[] }) => number;
  inheritedAgent: (wfId: string, step: StepConfig) => string;
  inheritedModel: (wfId: string, step: StepConfig) => string;
  inheritedEffort: (wfId: string, step: StepConfig) => EffortLevel;
  effectiveAgentForRow: (wfId: string, step: StepConfig | null) => string;
  /** The effort levels a harness accepts, from the backend agent catalog.
   *  Empty (hermes) means the picker must disable the control. */
  effortLevelsFor: (kind: string) => readonly EffortLevel[];
}

const Ctx = createContext<SettingsCtx | null>(null);
export function useSettings(): SettingsCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error('useSettings must be used within ProjectSettingsProvider');
  return c;
}

export function ProjectSettingsProvider({ children }: { children: React.ReactNode }) {
  const { navigate } = useNavigation();
  const { state: { currentProjectId, projects, providers }, dispatch: projDispatch } = useProject();
  const activeProject = projects.find(p => p.id === currentProjectId)!;
  const { reportError } = useErrorBus();

  const setProjects = (updater: (prev: Project[]) => Project[]) =>
    projDispatch({ type: 'UPDATE_PROJECTS', updater });
  const setCurrentProject = (id: string | null) => {
    if (id) projDispatch({ type: 'SET_CURRENT', id });
    else navigate({ kind: 'empty-state' });
  };

  const [isLoading, setIsLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<'general' | 'strategy' | 'overrides' | 'memory'>('general');
  const [status, setStatus] = useState<'idle' | 'saving' | 'success' | 'error'>('idle');
  const [errorMsg, setErrorMsg] = useState('');

  const [memories, setMemories] = useState<ProjectMemoryEntry[]>([]);
  const [isMemoriesLoading, setIsMemoriesLoading] = useState(false);
  const [editingMemory, setEditingMemory] = useState<ProjectMemoryEntry | null>(null);
  const [newMemKey, setNewMemKey] = useState('');
  const [newMemVal, setNewMemVal] = useState('');
  const [memError, setMemError] = useState('');

  const [projectName, setProjectName] = useState(activeProject.name);
  const [computeType, setComputeType] = useState(activeProject.compute_type || 'local');
  const [remoteHost, setRemoteHost] = useState(activeProject.remote_host || '');
  const [machines, setMachines] = useState<Machine[]>([]);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<'idle' | 'success' | 'error'>('idle');

  const [selectedRepos, setSelectedRepos] = useState<AvailableRepo[]>([]);
  const [originalRepos, setOriginalRepos] = useState<AvailableRepo[]>([]);
  const [isRepoModalOpen, setIsRepoModalOpen] = useState(false);
  const [repoSearch, setRepoSearch] = useState('');
  const [availableRepos, setAvailableRepos] = useState<AvailableRepo[]>([]);
  const [isLoadingRepos, setIsLoadingRepos] = useState(false);

  const [bootstrapStep, setBootstrapStep] = useState<'form' | 'bootstrapping' | 'strategy_proposal' | 'bootstrap_success' | 'error'>('form');
  const [bootstrapError, setBootstrapError] = useState('');

  const [healthData, setHealthData] = useState<RepoHealthStatus[] | null>(null);
  const [isLoadingHealth, setIsLoadingHealth] = useState(false);
  const [healthExpanded, setHealthExpanded] = useState(true);
  const [showHealthPanel, setShowHealthPanel] = useState(false);
  const [healthError, setHealthError] = useState('');

  const [defaultBranch, setDefaultBranch] = useState('');
  const [branchPrefix, setBranchPrefix] = useState('');
  const [testCommand, setTestCommand] = useState('');
  const [buildCommand, setBuildCommand] = useState('');
  const [coverageCommand, setCoverageCommand] = useState('');
  const [conventionsFile, setConventionsFile] = useState('');
  const [harnesses, setHarnesses] = useState<{ [key: string]: string }>({});
  const [validationGates, setValidationGates] = useState<string[]>([]);
  const [commandProbe, setCommandProbe] = useState<CommandProbeReport | null>(null);
  const [isProbingCommands, setIsProbingCommands] = useState(false);
  const [probeError, setProbeError] = useState('');
  const [probeNonce, setProbeNonce] = useState(0);
  const [prepareCommand, setPrepareCommand] = useState('');
  const [prTemplate, setPrTemplate] = useState('');
  const [conflictPolicy, setConflictPolicy] = useState('always_gate');
  const [featureLifecycle, setFeatureLifecycle] = useState('archive');

  const [dirtyWarningRepos, setDirtyWarningRepos] = useState<RepoDirtyStatus[]>([]);
  const [pendingActionAfterConfirm, setPendingActionAfterConfirm] = useState<'save' | 'delete' | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

  const [agentConfigs, setAgentConfigs] = useState<AgentConfigView[]>([]);
  const [isRefreshingAgents, setIsRefreshingAgents] = useState(false);

  const [defaultAgentKind, setDefaultAgentKind] = useState('');
  const [defaultModel, setDefaultModel] = useState('');
  const [defaultEffort, setDefaultEffort] = useState<EffortLevel | ''>('');
  const [defaultWorkflowId, setDefaultWorkflowId] = useState('');
  const [missingDefaultWorkflowId, setMissingDefaultWorkflowId] = useState<string | null>(null);
  const [defaultLoopIterations, setDefaultLoopIterations] = useState('');
  const [defaultMaxBudgetUsd, setDefaultMaxBudgetUsd] = useState('');
  const [availableModelsForDefault, setAvailableModelsForDefault] = useState<ConfigOptionValue[]>([]);
  const [isLoadingModelsForDefault, setIsLoadingModelsForDefault] = useState(false);
  const [artifactSubdir, setArtifactSubdir] = useState('artifacts/');
  const [commitArtifacts, setCommitArtifacts] = useState(false);
  const [reviewEntrypoint, setReviewEntrypoint] = useState('');
  const [syncResolverAgentKind, setSyncResolverAgentKind] = useState('');
  const [syncResolverModel, setSyncResolverModel] = useState('');
  const [syncResolverEffort, setSyncResolverEffort] = useState<EffortLevel | ''>('');
  const [availableModelsForSyncResolver, setAvailableModelsForSyncResolver] = useState<ConfigOptionValue[]>([]);
  const [isLoadingModelsForSyncResolver, setIsLoadingModelsForSyncResolver] = useState(false);
  const [extraWritablePaths, setExtraWritablePaths] = useState<string[]>([]);
  const [newExtraPath, setNewExtraPath] = useState('');

  const [workflows, setWorkflows] = useState<{ id: string; name: string; description: string; steps: StepConfig[] }[]>([]);
  const [workflowsLoaded, setWorkflowsLoaded] = useState(false);
  const [workflowsError, setWorkflowsError] = useState('');
  const [overrides, setOverrides] = useState<Record<string, OverrideRowValue>>({});
  const [isLoadingOverrides, setIsLoadingOverrides] = useState(false);
  const [overridesError, setOverridesError] = useState('');
  const [expandedWf, setExpandedWf] = useState<Record<string, boolean>>({});
  const [rowModels, setRowModels] = useState<Record<string, ConfigOptionValue[]>>({});
  const [rowModelsLoading, setRowModelsLoading] = useState<Record<string, boolean>>({});
  const [savedPulse, setSavedPulse] = useState<Record<string, boolean>>({});

  const overridesMachineId = computeType === 'remote' ? remoteHost : 'local';
  const overrideAgentKinds = agentConfigs
    .filter(a => a.enabled && a.available)
    .map(a => a.kind);

  const { agents: agentCatalog } = useAgentCatalog();
  const effortLevels = (kind: string) => effortLevelsFor(agentCatalog, kind);

  // Models are a harness's own namespace and the effort ladder is canonical but
  // not universally supported, so switching the resolver's harness drops the
  // pinned model and clamps the effort to what the new one accepts — otherwise
  // a level the previous harness offered lingers in a now-greyed control and is
  // persisted from it.
  const onSyncResolverAgentChange = (kind: string) => {
    setSyncResolverAgentKind(kind);
    setSyncResolverModel('');
    setSyncResolverEffort(e => reconcileEffort(e, effortLevels(kind || defaultAgentKind)));
  };

  const inheritedAgent = (workflowId: string, step: StepConfig): string => {
    const wfOv = overrides[ovKey(workflowId, WF_LEVEL)];
    return wfOv?.agent_kind || step.agent_kind || defaultAgentKind || '';
  };
  const inheritedModel = (workflowId: string, step: StepConfig): string => {
    const wfOv = overrides[ovKey(workflowId, WF_LEVEL)];
    return wfOv?.model || step.model || defaultModel || '';
  };
  // Shaped exactly like `inheritedModel`, with one difference: the chain has a
  // known terminal value. The engine falls back to `EffortLevel::DEFAULT`
  // (`high`) when nothing pins an effort, so a row with nothing above it shows
  // `high` rather than a blank — the placeholder states what will actually run.
  const inheritedEffort = (workflowId: string, step: StepConfig): EffortLevel => {
    const wfOv = overrides[ovKey(workflowId, WF_LEVEL)];
    return wfOv?.effort || step.effort || defaultEffort || DEFAULT_EFFORT;
  };
  const effectiveAgentForRow = (workflowId: string, step: StepConfig | null): string => {
    if (step === null) return overrides[ovKey(workflowId, WF_LEVEL)]?.agent_kind || defaultAgentKind || '';
    return overrides[ovKey(workflowId, step.id)]?.agent_kind || inheritedAgent(workflowId, step);
  };

  const probeModels = async (key: string, agentKind: string) => {
    if (!agentKind) { setRowModels(prev => ({ ...prev, [key]: [] })); return; }
    setRowModelsLoading(prev => ({ ...prev, [key]: true }));
    try {
      const models = await getAgentModels(overridesMachineId, agentKind);
      setRowModels(prev => ({ ...prev, [key]: models }));
    } catch { setRowModels(prev => ({ ...prev, [key]: [] })); }
    finally { setRowModelsLoading(prev => ({ ...prev, [key]: false })); }
  };

  const toggleWorkflowExpanded = (wf: { id: string; steps: StepConfig[] }) => {
    const willExpand = !expandedWf[wf.id];
    setExpandedWf(prev => ({ ...prev, [wf.id]: willExpand }));
    if (willExpand) {
      const wfAgent = effectiveAgentForRow(wf.id, null);
      if (wfAgent) probeModels(ovKey(wf.id, WF_LEVEL), wfAgent);
      for (const step of wf.steps) {
        if (step.kind === 'gate') continue;
        const agent = effectiveAgentForRow(wf.id, step);
        if (agent) probeModels(ovKey(wf.id, step.id), agent);
      }
    }
  };

  const persistOverride = async (workflowId: string, stepId: string, next: OverrideRowValue) => {
    const key = ovKey(workflowId, stepId);
    try {
      await setWorkflowOverride({ projectId: activeProject.id, workflowId, stepId: stepId || null, agentKind: next.agent_kind, model: next.model, effort: next.effort });
      setSavedPulse(prev => ({ ...prev, [key]: true }));
      setTimeout(() => setSavedPulse(prev => ({ ...prev, [key]: false })), 1400);
    } catch (err) { setOverridesError(formatError(err)); }
  };

  const handleAgentChange = (workflowId: string, stepId: string, step: StepConfig | null, agentKind: string) => {
    const key = ovKey(workflowId, stepId);
    const current = overrides[key] ?? EMPTY_ROW;
    // The model belongs to the previous harness's namespace, so it is dropped.
    // The effort ladder is canonical across agents, but the new effective
    // harness may not accept the current rung — clamp it down (or clear it)
    // rather than keep a level the picker would show but the agent can't run.
    const probeAgent = agentKind || (step ? inheritedAgent(workflowId, step) : (defaultAgentKind || ''));
    const nextEffort = reconcileEffort(current.effort ?? '', effortLevels(probeAgent)) || null;
    const next: OverrideRowValue = { agent_kind: agentKind || null, model: null, effort: nextEffort };
    setOverrides(prev => ({ ...prev, [key]: next }));
    probeModels(key, probeAgent);
    persistOverride(workflowId, stepId, next);
  };

  const handleModelChange = (workflowId: string, stepId: string, model: string) => {
    const key = ovKey(workflowId, stepId);
    const current = overrides[key] ?? EMPTY_ROW;
    const next: OverrideRowValue = { ...current, model: model || null };
    setOverrides(prev => ({ ...prev, [key]: next }));
    persistOverride(workflowId, stepId, next);
  };

  const handleEffortChange = (workflowId: string, stepId: string, effort: EffortLevel | '') => {
    const key = ovKey(workflowId, stepId);
    const current = overrides[key] ?? EMPTY_ROW;
    const next: OverrideRowValue = { ...current, effort: effort || null };
    setOverrides(prev => ({ ...prev, [key]: next }));
    persistOverride(workflowId, stepId, next);
  };

  const handleClearRow = (workflowId: string, stepId: string) => {
    const key = ovKey(workflowId, stepId);
    setOverrides(prev => ({ ...prev, [key]: EMPTY_ROW }));
    persistOverride(workflowId, stepId, EMPTY_ROW);
  };

  const workflowOverrideCount = (wf: { id: string; steps: StepConfig[] }): number => {
    let n = 0;
    if (isOverrideActive(overrides[ovKey(wf.id, WF_LEVEL)])) n++;
    for (const s of wf.steps) {
      if (isOverrideActive(overrides[ovKey(wf.id, s.id)])) n++;
    }
    return n;
  };

  // Both tabs read the same list — Strategy for the default-workflow picker,
  // Overrides for its per-step table — so it is fetched once, outside the
  // override fetch that only one of them needs.
  useEffect(() => {
    if (activeTab !== 'strategy' && activeTab !== 'overrides') return;
    let cancelled = false;
    (async () => {
      setWorkflowsError('');
      try {
        const list = (await listWorkflows()) ?? [];
        if (cancelled) return;
        setWorkflows(list.map(w => ({ id: w.id, name: w.name, description: w.description, steps: w.steps ?? [] })));
        setWorkflowsLoaded(true);
      } catch (err) { if (!cancelled) setWorkflowsError(formatError(err)); }
    })();
    return () => { cancelled = true; };
  }, [activeTab]);

  useEffect(() => {
    if (!workflowsLoaded) return;
    const { selected, dangling } = reconcileDefaultWorkflow(defaultWorkflowId, workflows);
    if (dangling) { setMissingDefaultWorkflowId(dangling); setDefaultWorkflowId(selected); }
  }, [workflowsLoaded, workflows, defaultWorkflowId]);

  const chooseDefaultWorkflow = (id: string) => {
    setDefaultWorkflowId(id);
    setMissingDefaultWorkflowId(null);
  };

  useEffect(() => {
    if (activeTab === 'overrides') {
      (async () => {
        setIsLoadingOverrides(true); setOverridesError('');
        try {
          const ovList = await getWorkflowOverrides(activeProject.id);
          const map: Record<string, OverrideRowValue> = {};
          for (const ov of ovList) map[ovKey(ov.workflow_id, ov.step_id ?? WF_LEVEL)] = { agent_kind: ov.agent_kind ?? null, model: ov.model ?? null, effort: ov.effort ?? null };
          setOverrides(map);
          const toExpand: Record<string, boolean> = {};
          for (const ov of ovList) toExpand[ov.workflow_id] = true;
          setExpandedWf(toExpand);
          for (const ov of ovList) if (ov.agent_kind) probeModels(ovKey(ov.workflow_id, ov.step_id ?? WF_LEVEL), ov.agent_kind);
        } catch (err) { setOverridesError(formatError(err)); }
        finally { setIsLoadingOverrides(false); }
      })();
    }
  }, [activeTab, activeProject.id, overridesMachineId]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!defaultAgentKind) { setAvailableModelsForDefault([]); return; }
      setIsLoadingModelsForDefault(true);
      try {
        const machineId = computeType === 'remote' ? remoteHost : 'local';
        const models = await getAgentModels(machineId, defaultAgentKind);
        if (!cancelled) setAvailableModelsForDefault(models);
      } catch { if (!cancelled) setAvailableModelsForDefault([]); }
      finally { if (!cancelled) setIsLoadingModelsForDefault(false); }
    })();
    return () => { cancelled = true; };
  }, [defaultAgentKind, computeType, remoteHost]);

  // The resolver row pins a model for whichever harness it will actually run
  // under, so the probe follows the effective kind rather than the pinned one:
  // a row that inherits the harness can still pin a model for it.
  const effectiveSyncResolverAgent = syncResolverAgentKind || defaultAgentKind;
  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!effectiveSyncResolverAgent) { setAvailableModelsForSyncResolver([]); return; }
      setIsLoadingModelsForSyncResolver(true);
      try {
        const machineId = computeType === 'remote' ? remoteHost : 'local';
        const models = await getAgentModels(machineId, effectiveSyncResolverAgent);
        if (!cancelled) setAvailableModelsForSyncResolver(models);
      } catch { if (!cancelled) setAvailableModelsForSyncResolver([]); }
      finally { if (!cancelled) setIsLoadingModelsForSyncResolver(false); }
    })();
    return () => { cancelled = true; };
  }, [effectiveSyncResolverAgent, computeType, remoteHost]);

  useEffect(() => { setConnectionStatus('idle'); }, [remoteHost]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await listMachines();
        if (!cancelled) setMachines(list ?? []);
      } catch (err) { reportError(err, { kind: 'internal' }); }
    })();
    return () => { cancelled = true; };
  }, []);

  const fetchAgentConfigs = async (refresh = false) => {
    const machineId = computeType === 'remote' ? remoteHost : 'local';
    if (computeType === 'remote' && !remoteHost) { setAgentConfigs([]); return; }
    if (refresh) setIsRefreshingAgents(true);
    try {
      const configs = await getAgentConfigs(machineId, refresh);
      setAgentConfigs(configs);
    } catch (err) {
      console.warn('No agent configs for machine:', machineId, formatError(err));
      setAgentConfigs([]);
    } finally { if (refresh) setIsRefreshingAgents(false); }
  };

  useEffect(() => { fetchAgentConfigs(); }, [computeType, remoteHost]);

  const refreshCommandProbe = () => setProbeNonce(n => n + 1);

  // Probe the configured commands where they are authored (HB6). Runs against
  // the *project's* machine — the backend picks it from the compute type, so a
  // remote project is never answered with the laptop's PATH — and re-runs as
  // the commands change, which is what makes a mistyped binary visible without
  // leaving the panel. Debounced because a keystroke is not a question worth a
  // `command -v` round trip.
  //
  // Every failure mode ends in an indicator, never a block: `handleSave` does
  // not read any of this state.
  useEffect(() => {
    if (activeTab !== 'strategy' || isLoading) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      (async () => {
        setIsProbingCommands(true);
        try {
          const report = await probeProjectCommands({
            projectId: activeProject.id,
            prepareCommand,
            testCommand,
            harnesses,
          });
          if (!cancelled) { setCommandProbe(report); setProbeError(''); }
        } catch (err) {
          if (!cancelled) { setCommandProbe(null); setProbeError(formatError(err)); }
        } finally { if (!cancelled) setIsProbingCommands(false); }
      })();
    }, PROBE_DEBOUNCE_MS);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [activeTab, isLoading, activeProject.id, computeType, remoteHost, prepareCommand, testCommand, harnesses, probeNonce]);

  const fetchWorkspaceHealth = async () => {
    setIsLoadingHealth(true); setHealthError('');
    try {
      const data = await getWorkspaceHealth(activeProject.id);
      setHealthData(data); setShowHealthPanel(true); setHealthExpanded(true);
    } catch (err) {
      setHealthError(formatError(err)); setHealthData([]); setShowHealthPanel(false);
    } finally { setIsLoadingHealth(false); }
  };

  useEffect(() => {
    (async () => {
      setIsLoading(true);
      try {
        const res = await getProposedStrategy(activeProject.id);
        if (res) {
          setDefaultBranch(res.worktree_strategy.default_branch);
          setBranchPrefix(res.worktree_strategy.branch_prefix);
          setTestCommand(res.worktree_strategy.test_command || '');
          setBuildCommand(res.worktree_strategy.build_command || '');
          setCoverageCommand(res.worktree_strategy.coverage_command || '');
          setConventionsFile(res.worktree_strategy.conventions_file || '');
          setHarnesses(res.worktree_strategy.harnesses || {});
          setValidationGates(res.worktree_strategy.validation_gates || []);
          setPrepareCommand(res.worktree_strategy.prepare_command || '');
          setPrTemplate(res.worktree_strategy.pr_template || '');
          setConflictPolicy(res.conflict_policy);
          setFeatureLifecycle(res.feature_lifecycle);
          setDefaultAgentKind(res.default_agent_kind || '');
          setDefaultModel(res.default_model || '');
          setDefaultEffort(res.default_effort || '');
          setDefaultWorkflowId(res.default_workflow_id || '');
          setDefaultLoopIterations(res.default_loop_iterations != null ? String(res.default_loop_iterations) : '');
          setDefaultMaxBudgetUsd(res.default_max_budget_usd != null ? String(res.default_max_budget_usd) : '');
          setArtifactSubdir(res.artifact_subdir || 'artifacts/');
          setCommitArtifacts(Boolean(res.commit_artifacts));
          setReviewEntrypoint(res.review_entrypoint || '');
          setSyncResolverAgentKind(res.sync_resolver_agent_kind || '');
          setSyncResolverModel(res.sync_resolver_model || '');
          setSyncResolverEffort(res.sync_resolver_effort || '');
          setExtraWritablePaths(res.worktree_strategy.extra_writable_paths || []);
        }
        const reposRes = await getRepositoriesForProject(activeProject.id);
        const mappedRepos = reposRes.map(r => ({ path: r.repo_path, providerId: r.provider_id }));
        setSelectedRepos(mappedRepos); setOriginalRepos(mappedRepos);
      } catch (err) {
        setErrorMsg(formatError(err)); setStatus('error');
        setSelectedRepos([]); setOriginalRepos([]);
      } finally { setIsLoading(false); }
    })();
    if (activeProject.status === 'idle') { setShowHealthPanel(true); fetchWorkspaceHealth(); }
  }, [activeProject.id]);

  useEffect(() => {
    if (activeTab === 'memory') fetchMemories();
  }, [activeTab, activeProject.id]);

  const fetchMemories = async () => {
    setIsMemoriesLoading(true); setMemError('');
    try {
      const list = await listProjectMemory(activeProject.id);
      setMemories(list ?? []);
    } catch (err) { setMemError(formatError(err)); }
    finally { setIsMemoriesLoading(false); }
  };

  const handleSaveMemory = async (e: React.FormEvent) => {
    e.preventDefault();
    const key = newMemKey.trim(); const value = newMemVal.trim();
    if (!key || !value) { setMemError('Key and Value cannot be empty.'); return; }
    try {
      await upsertProjectMemory(activeProject.id, key, value, editingMemory ? editingMemory.source : 'human', editingMemory ? editingMemory.id : null);
      setNewMemKey(''); setNewMemVal(''); setEditingMemory(null);
      fetchMemories();
    } catch (err) { setMemError(formatError(err)); }
  };

  const handleDeleteMemory = async (id: string) => {
    try { await deleteProjectMemory(id); fetchMemories(); }
    catch (err) { setMemError(formatError(err)); }
  };

  const handleEditMemoryClick = (entry: ProjectMemoryEntry) => { setEditingMemory(entry); setNewMemKey(entry.key); setNewMemVal(entry.value); };
  const handleCancelEdit = () => { setEditingMemory(null); setNewMemKey(''); setNewMemVal(''); };

  const fetchAllReposFromProviders = async () => {
    if (providers.length === 0) return;
    setIsLoadingRepos(true);
    try {
      const allRepos = await Promise.all(providers.map(async p => {
        try {
          const repos = await fetchProviderRepos(p.id);
          return repos.map(r => ({ path: r, providerId: p.id }));
        } catch (err) { reportError(err, { kind: 'provider' }); return []; }
      }));
      const seen = new Set<string>();
      const uniqueRepos: AvailableRepo[] = [];
      for (const r of allRepos.flat()) { if (!seen.has(r.path)) { seen.add(r.path); uniqueRepos.push(r); } }
      setAvailableRepos(uniqueRepos);
    } catch (err) { setErrorMsg(formatError(err)); setStatus('error'); setAvailableRepos([]); }
    finally { setIsLoadingRepos(false); }
  };

  const toggleRepo = (repo: AvailableRepo) => setSelectedRepos(prev => prev.some(r => r.path === repo.path) ? prev.filter(r => r.path !== repo.path) : [...prev, repo]);

  const handleTestConnection = async () => {
    if (!remoteHost) return;
    setIsTestingConnection(true); setConnectionStatus('idle');
    try { await testMachineConnection(remoteHost); setConnectionStatus('success'); }
    catch (err) { setConnectionStatus('error'); setErrorMsg('Connection test failed: ' + formatError(err)); setStatus('error'); }
    finally { setIsTestingConnection(false); }
  };

  const checkDirtyRepositories = async (repos: AvailableRepo[]): Promise<RepoDirtyStatus[]> => {
    if (repos.length === 0) return [];
    try {
      const res = await checkReposDirty(activeProject.id, repos.map(r => r.path));
      return res.filter(r => r.has_uncommitted || r.has_unpushed);
    } catch (err) { reportError(err, { kind: 'internal' }); return []; }
  };

  // A tick left behind by a deleted harness is not a declaration: the engine
  // drops names its `harnesses` map no longer defines when it resolves tier 2,
  // so persisting one would keep alive a selection nothing can honour.
  const gatesToPersist = () =>
    validationGates.filter(g => Object.prototype.hasOwnProperty.call(harnesses, g));

  const saveAllSettings = async () => {
    const machineId = computeType === 'remote' ? remoteHost : 'local';
    if (machineId) {
      try { await writeAgentConfigs(machineId, agentConfigs.map(a => ({ kind: a.kind, enabled: a.enabled }))); }
      catch (err) { reportError(err, { kind: 'validation' }); }
    }
    await updateProject(activeProject.id, { name: projectName, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null, repos: selectedRepos.map(r => ({ repo_path: r.path, provider_id: r.providerId })) });
await saveProjectSettings(activeProject.id, { default_branch: defaultBranch, branch_prefix: branchPrefix, test_command: testCommand || null, build_command: buildCommand || null, coverage_command: coverageCommand || null, conventions_file: conventionsFile || null, pr_template: prTemplate || null, harnesses: Object.keys(harnesses).length > 0 ? harnesses : null, validation_gates: gatesToPersist(), prepare_command: prepareCommand || null, extra_writable_paths: extraWritablePaths.length > 0 ? extraWritablePaths : null, conflict_policy: conflictPolicy, feature_lifecycle: featureLifecycle, default_agent_kind: defaultAgentKind || null, default_model: defaultModel || null, default_effort: defaultEffort || null, default_workflow_id: defaultWorkflowId || null, default_loop_iterations: defaultLoopIterations.trim() ? parseInt(defaultLoopIterations, 10) : null, default_max_budget_usd: defaultMaxBudgetUsd.trim() ? parseFloat(defaultMaxBudgetUsd) : null, artifact_subdir: artifactSubdir || 'artifacts/', commit_artifacts: commitArtifacts, review_entrypoint: reviewEntrypoint.trim() || null, sync_resolver_agent_kind: syncResolverAgentKind || null, sync_resolver_model: syncResolverModel || null, sync_resolver_effort: syncResolverEffort || null });
  };

  const handleSave = async () => {
    setStatus('saving'); setErrorMsg('');
    const reposChanged = selectedRepos.length !== originalRepos.length || selectedRepos.some(r => !originalRepos.some(o => o.path === r.path));
    const computeChanged = computeType !== activeProject.compute_type || remoteHost !== activeProject.remote_host;
    const isCurrentlyFailedOrBootstrapping = activeProject.status === 'error' || activeProject.status === 'bootstrapping';
    const machineId = computeType === 'remote' ? remoteHost : 'local';
    if (machineId) {
      try { await writeAgentConfigs(machineId, agentConfigs.map(a => ({ kind: a.kind, enabled: a.enabled }))); }
      catch (err) { reportError(err, { kind: 'validation' }); }
    }
    if (reposChanged || computeChanged || isCurrentlyFailedOrBootstrapping) {
      const removedRepos = originalRepos.filter(o => !selectedRepos.some(s => s.path === o.path));
      if (removedRepos.length > 0) {
        const dirtyList = await checkDirtyRepositories(removedRepos);
        if (dirtyList.length > 0) { setDirtyWarningRepos(dirtyList); setPendingActionAfterConfirm('save'); setStatus('idle'); return; }
      }
      await proceedWithReBootstrap();
    } else {
      try {
        await updateProject(activeProject.id, { name: projectName, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null, repos: selectedRepos.map(r => ({ repo_path: r.path, provider_id: r.providerId })) });
        await saveProjectSettings(activeProject.id, { default_branch: defaultBranch, branch_prefix: branchPrefix, test_command: testCommand || null, build_command: buildCommand || null, coverage_command: coverageCommand || null, conventions_file: conventionsFile || null, pr_template: prTemplate || null, harnesses: Object.keys(harnesses).length > 0 ? harnesses : null, validation_gates: gatesToPersist(), prepare_command: prepareCommand || null, extra_writable_paths: extraWritablePaths.length > 0 ? extraWritablePaths : null, conflict_policy: conflictPolicy, feature_lifecycle: featureLifecycle, default_agent_kind: defaultAgentKind || null, default_model: defaultModel || null, default_effort: defaultEffort || null, default_workflow_id: defaultWorkflowId || null, default_loop_iterations: defaultLoopIterations.trim() ? parseInt(defaultLoopIterations, 10) : null, default_max_budget_usd: defaultMaxBudgetUsd.trim() ? parseFloat(defaultMaxBudgetUsd) : null, artifact_subdir: artifactSubdir || 'artifacts/', commit_artifacts: commitArtifacts, review_entrypoint: reviewEntrypoint.trim() || null, sync_resolver_agent_kind: syncResolverAgentKind || null, sync_resolver_model: syncResolverModel || null, sync_resolver_effort: syncResolverEffort || null });
        // Keep `compute_type` / `remote_host` in sync with the DB so the
        // Settings tab doesn't fall back to "Local Compute" the next
        // time the user reopens it. Mirrors the re-bootstrap save path
        // below (line ~531).
        setProjects(prev => prev.map(p => p.id === activeProject.id ? { ...p, name: projectName, repos: selectedRepos.length, nodes: computeType === 'local' ? 4 : 8, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null } : p));
        setStatus('success'); setOriginalRepos(selectedRepos);
        setTimeout(() => setStatus('idle'), 1500);
      } catch (err) { setStatus('error'); setErrorMsg(formatError(err)); }
    }
  };

  const proceedWithReBootstrap = async () => {
    setBootstrapStep('bootstrapping'); setBootstrapError('');
    // Preserves a value the user edited in a prior strategy_proposal round
    // before this call's fetches (which may reset the state) run.
    const currentDefaultBranch = defaultBranch;
    const currentBranchPrefix = branchPrefix;
    const currentTestCommand = testCommand;
    const currentPrTemplate = prTemplate;
    try {
      const existing = await getProposedStrategy(activeProject.id);
      await updateProject(activeProject.id, { name: projectName, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null, repos: selectedRepos.map(r => ({ repo_path: r.path, provider_id: r.providerId })) });
      const strategy = await bootstrapProject(activeProject.id);
      const ext = existing?.worktree_strategy;
      setDefaultBranch(currentDefaultBranch || ext?.default_branch || strategy.default_branch);
      setBranchPrefix(currentBranchPrefix || ext?.branch_prefix || strategy.branch_prefix);
      setTestCommand(currentTestCommand || ext?.test_command || strategy.test_command || '');
      setPrTemplate(currentPrTemplate || ext?.pr_template || strategy.pr_template || '');
      setBootstrapStep('strategy_proposal');
    } catch (err) { setBootstrapStep('error'); setBootstrapError(formatError(err)); }
  };

  const handleApproveStrategy = async () => {
    try {
      await saveAllSettings();
      setProjects(prev => prev.map(p => p.id === activeProject.id ? { ...p, name: projectName, status: 'idle', repos: selectedRepos.length, nodes: computeType === 'local' ? 4 : 8, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null } : p));
      setBootstrapStep('bootstrap_success');
    } catch (err) { setBootstrapStep('error'); setBootstrapError(formatError(err)); }
  };

  const handleDeleteClick = async () => {
    const dirtyList = await checkDirtyRepositories(selectedRepos);
    if (dirtyList.length > 0) { setDirtyWarningRepos(dirtyList); setPendingActionAfterConfirm('delete'); }
    else setShowDeleteConfirm(true);
  };

  const proceedWithDelete = async () => {
    setIsLoading(true);
    try {
      await deleteProject(activeProject.id);
      setProjects(prev => prev.filter(p => p.id !== activeProject.id));
      setCurrentProject(null);
      navigate({ kind: 'empty-state' });
    } catch (err) { setErrorMsg('Failed to delete workspace: ' + formatError(err)); setStatus('error'); }
    finally { setIsLoading(false); setShowDeleteConfirm(false); setDirtyWarningRepos([]); setPendingActionAfterConfirm(null); }
  };

  const value: SettingsCtx = {
    activeProject, navigate,
    isLoading, activeTab, setActiveTab, status, errorMsg,
    memories, isMemoriesLoading, editingMemory, setEditingMemory, newMemKey, setNewMemKey, newMemVal, setNewMemVal, memError,
    projectName, setProjectName, computeType, setComputeType, remoteHost, setRemoteHost, machines,
    isTestingConnection, connectionStatus, selectedRepos, originalRepos,
    isRepoModalOpen, setIsRepoModalOpen, repoSearch, setRepoSearch, availableRepos, isLoadingRepos,
    bootstrapStep, setBootstrapStep, bootstrapError,
    healthData, isLoadingHealth, healthExpanded, setHealthExpanded, showHealthPanel, healthError,
    defaultBranch, setDefaultBranch, branchPrefix, setBranchPrefix, testCommand, setTestCommand,
    buildCommand, setBuildCommand, coverageCommand, setCoverageCommand, conventionsFile, setConventionsFile,
    harnesses, setHarnesses, validationGates, setValidationGates,
    commandProbe, isProbingCommands, probeError, refreshCommandProbe,
    prepareCommand, setPrepareCommand, prTemplate, setPrTemplate, conflictPolicy, setConflictPolicy,
    featureLifecycle, setFeatureLifecycle, defaultAgentKind, setDefaultAgentKind, defaultModel, setDefaultModel,
    defaultEffort, setDefaultEffort,
    defaultWorkflowId, setDefaultWorkflowId: chooseDefaultWorkflow, missingDefaultWorkflowId,
    defaultLoopIterations, setDefaultLoopIterations, availableModelsForDefault, isLoadingModelsForDefault,
    defaultMaxBudgetUsd, setDefaultMaxBudgetUsd,
    agentConfigs, setAgentConfigs, isRefreshingAgents, artifactSubdir, setArtifactSubdir, commitArtifacts, setCommitArtifacts,
    reviewEntrypoint, setReviewEntrypoint,
    syncResolverAgentKind, setSyncResolverAgentKind: onSyncResolverAgentChange,
    syncResolverModel, setSyncResolverModel,
    syncResolverEffort, setSyncResolverEffort,
    availableModelsForSyncResolver, isLoadingModelsForSyncResolver,
    extraWritablePaths, setExtraWritablePaths, newExtraPath, setNewExtraPath,
    dirtyWarningRepos, setDirtyWarningRepos, pendingActionAfterConfirm, setPendingActionAfterConfirm, showDeleteConfirm, setShowDeleteConfirm,
    workflows, workflowsLoaded, workflowsError,
    overrides, setOverrides, isLoadingOverrides, overridesError, expandedWf, setExpandedWf,
    rowModels, rowModelsLoading, savedPulse, overrideAgentKinds, overridesMachineId,
    handleSave, handleDeleteClick, proceedWithReBootstrap, proceedWithDelete, handleApproveStrategy,
    handleSaveMemory, handleDeleteMemory, handleEditMemoryClick, handleCancelEdit,
    fetchAllReposFromProviders, toggleRepo, handleTestConnection, fetchWorkspaceHealth, fetchAgentConfigs,
    toggleWorkflowExpanded, handleAgentChange, handleModelChange, handleEffortChange, handleClearRow, workflowOverrideCount,
    inheritedAgent, inheritedModel, inheritedEffort, effectiveAgentForRow,
    effortLevelsFor: effortLevels,
  };

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
