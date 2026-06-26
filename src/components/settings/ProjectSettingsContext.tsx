import { createContext, useContext, useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ConfigOptionValue, ProjectMemoryEntry, WorkflowOverride, StepConfig, Machine, Project, WorktreeStrategy, ProjectSettingsData } from '../../types';
import { getAgentModels } from '../../lib/agentModels';
import { formatError } from '../../lib/errors';
import { useErrorBus } from '../../lib/errorBus';
import { saveProjectSettings } from '../../lib/project';
import { useNavigation, useProject } from '../../context';

export interface AvailableRepo { path: string; providerId: string; }
export interface RepoDirtyStatus { repo_path: string; has_uncommitted: boolean; has_unpushed: boolean; }
export interface WorktreeInfo { path: string; branch: string | null; is_locked: boolean; }
export interface RepoHealthStatus {
  repo_path: string; is_cloned: boolean; head_branch: string | null;
  worktrees: WorktreeInfo[]; has_uncommitted: boolean; has_unpushed: boolean;
}
export interface AgentConfigView { kind: string; enabled: boolean; available: boolean; install_command: string; }

export const WF_LEVEL = '';
export const ovKey = (workflowId: string, stepId: string) => `${workflowId}::${stepId}`;

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
  prTemplate: string; setPrTemplate: (v: string) => void;
  conflictPolicy: string; setConflictPolicy: (v: string) => void;
  featureLifecycle: string; setFeatureLifecycle: (v: string) => void;
  defaultAgentKind: string; setDefaultAgentKind: (v: string) => void;
  defaultModel: string; setDefaultModel: (v: string) => void;
  defaultLoopIterations: string; setDefaultLoopIterations: (v: string) => void;
  availableModelsForDefault: ConfigOptionValue[]; isLoadingModelsForDefault: boolean;
  agentConfigs: AgentConfigView[]; setAgentConfigs: (v: AgentConfigView[]) => void;
  isRefreshingAgents: boolean;
  artifactSubdir: string; setArtifactSubdir: (v: string) => void;
  commitArtifacts: boolean; setCommitArtifacts: (v: boolean) => void;
  // warning modals
  dirtyWarningRepos: RepoDirtyStatus[];
  setDirtyWarningRepos: (v: RepoDirtyStatus[]) => void;
  pendingActionAfterConfirm: 'save' | 'delete' | null;
  setPendingActionAfterConfirm: (v: 'save' | 'delete' | null) => void;
  showDeleteConfirm: boolean; setShowDeleteConfirm: (v: boolean) => void;
  // overrides
  workflows: { id: string; name: string; description: string; steps: StepConfig[] }[];
  overrides: Record<string, { agent_kind: string | null; model: string | null }>;
  setOverrides: (v: Record<string, { agent_kind: string | null; model: string | null }>) => void;
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
  handleClearRow: (wfId: string, stepId: string) => void;
  workflowOverrideCount: (wf: { id: string; steps: StepConfig[] }) => number;
  inheritedAgent: (wfId: string, step: StepConfig) => string;
  inheritedModel: (wfId: string, step: StepConfig) => string;
  effectiveAgentForRow: (wfId: string, step: StepConfig | null) => string;
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

  const [defaultBranch, setDefaultBranch] = useState('main');
  const [branchPrefix, setBranchPrefix] = useState('demeteo/features/');
  const [testCommand, setTestCommand] = useState('');
  const [buildCommand, setBuildCommand] = useState('');
  const [coverageCommand, setCoverageCommand] = useState('');
  const [conventionsFile, setConventionsFile] = useState('');
  const [harnesses, setHarnesses] = useState<{ [key: string]: string }>({});
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
  const [defaultLoopIterations, setDefaultLoopIterations] = useState('');
  const [availableModelsForDefault, setAvailableModelsForDefault] = useState<ConfigOptionValue[]>([]);
  const [isLoadingModelsForDefault, setIsLoadingModelsForDefault] = useState(false);
  const [artifactSubdir, setArtifactSubdir] = useState('artifacts/');
  const [commitArtifacts, setCommitArtifacts] = useState(false);

  const [workflows, setWorkflows] = useState<{ id: string; name: string; description: string; steps: StepConfig[] }[]>([]);
  const [overrides, setOverrides] = useState<Record<string, { agent_kind: string | null; model: string | null }>>({});
  const [isLoadingOverrides, setIsLoadingOverrides] = useState(false);
  const [overridesError, setOverridesError] = useState('');
  const [expandedWf, setExpandedWf] = useState<Record<string, boolean>>({});
  const [rowModels, setRowModels] = useState<Record<string, ConfigOptionValue[]>>({});
  const [rowModelsLoading, setRowModelsLoading] = useState<Record<string, boolean>>({});
  const [savedPulse, setSavedPulse] = useState<Record<string, boolean>>({});

  const overridesMachineId = computeType === 'remote' ? remoteHost : 'local';
  const overrideAgentKinds = agentConfigs
    .filter(a => a.enabled && a.available && a.kind !== 'antigravity')
    .map(a => a.kind);

  const inheritedAgent = (workflowId: string, step: StepConfig): string => {
    const wfOv = overrides[ovKey(workflowId, WF_LEVEL)];
    return wfOv?.agent_kind || step.agent_kind || defaultAgentKind || '';
  };
  const inheritedModel = (workflowId: string, step: StepConfig): string => {
    const wfOv = overrides[ovKey(workflowId, WF_LEVEL)];
    return wfOv?.model || step.model || defaultModel || '';
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

  const persistOverride = async (workflowId: string, stepId: string, next: { agent_kind: string | null; model: string | null }) => {
    const key = ovKey(workflowId, stepId);
    try {
      await invoke('set_workflow_override', { projectId: activeProject.id, workflowId, stepId: stepId || null, agentKind: next.agent_kind, model: next.model });
      setSavedPulse(prev => ({ ...prev, [key]: true }));
      setTimeout(() => setSavedPulse(prev => ({ ...prev, [key]: false })), 1400);
    } catch (err) { setOverridesError(formatError(err)); }
  };

  const handleAgentChange = (workflowId: string, stepId: string, step: StepConfig | null, agentKind: string) => {
    const next = { agent_kind: agentKind || null, model: null };
    const key = ovKey(workflowId, stepId);
    setOverrides(prev => ({ ...prev, [key]: next }));
    const probeAgent = agentKind || (step ? inheritedAgent(workflowId, step) : (defaultAgentKind || ''));
    probeModels(key, probeAgent);
    persistOverride(workflowId, stepId, next);
  };

  const handleModelChange = (workflowId: string, stepId: string, model: string) => {
    const key = ovKey(workflowId, stepId);
    const current = overrides[key] ?? { agent_kind: null, model: null };
    const next = { agent_kind: current.agent_kind, model: model || null };
    setOverrides(prev => ({ ...prev, [key]: next }));
    persistOverride(workflowId, stepId, next);
  };

  const handleClearRow = (workflowId: string, stepId: string) => {
    const key = ovKey(workflowId, stepId);
    const next = { agent_kind: null, model: null };
    setOverrides(prev => ({ ...prev, [key]: next }));
    persistOverride(workflowId, stepId, next);
  };

  const workflowOverrideCount = (wf: { id: string; steps: StepConfig[] }): number => {
    let n = 0;
    if (overrides[ovKey(wf.id, WF_LEVEL)]?.agent_kind || overrides[ovKey(wf.id, WF_LEVEL)]?.model) n++;
    for (const s of wf.steps) {
      const o = overrides[ovKey(wf.id, s.id)];
      if (o?.agent_kind || o?.model) n++;
    }
    return n;
  };

  useEffect(() => {
    if (activeTab === 'overrides') {
      (async () => {
        setIsLoadingOverrides(true); setOverridesError('');
        try {
          const [wfList, ovList] = await Promise.all([
            invoke<{ id: string; name: string; description: string; steps: StepConfig[] }[]>('workflow_list'),
            invoke<WorkflowOverride[]>('get_workflow_overrides', { projectId: activeProject.id }),
          ]);
          setWorkflows(wfList.map(w => ({ id: w.id, name: w.name, description: w.description, steps: w.steps ?? [] })));
          const map: Record<string, { agent_kind: string | null; model: string | null }> = {};
          for (const ov of ovList) map[ovKey(ov.workflow_id, ov.step_id ?? WF_LEVEL)] = { agent_kind: ov.agent_kind ?? null, model: ov.model ?? null };
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

  useEffect(() => { setConnectionStatus('idle'); }, [remoteHost]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list: Machine[] = await invoke('get_machines');
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
      const configs = await invoke<AgentConfigView[]>('get_agent_configs', { machineId, refresh });
      setAgentConfigs(configs);
    } catch (err) {
      console.warn('No agent configs for machine:', machineId, formatError(err));
      setAgentConfigs([]);
    } finally { if (refresh) setIsRefreshingAgents(false); }
  };

  useEffect(() => { fetchAgentConfigs(); }, [computeType, remoteHost]);

  const fetchWorkspaceHealth = async () => {
    setIsLoadingHealth(true); setHealthError('');
    try {
      const data = await invoke<RepoHealthStatus[]>('get_workspace_health', { projectId: activeProject.id });
      setHealthData(data); setShowHealthPanel(true); setHealthExpanded(true);
    } catch (err) {
      setHealthError(formatError(err)); setHealthData([]); setShowHealthPanel(false);
    } finally { setIsLoadingHealth(false); }
  };

  useEffect(() => {
    (async () => {
      setIsLoading(true);
      try {
        const res = await invoke<ProjectSettingsData | null>('get_proposed_strategy', { projectId: activeProject.id });
        if (res) {
          setDefaultBranch(res.worktree_strategy.default_branch);
          setBranchPrefix(res.worktree_strategy.branch_prefix);
          setTestCommand(res.worktree_strategy.test_command || '');
          setBuildCommand(res.worktree_strategy.build_command || '');
          setCoverageCommand(res.worktree_strategy.coverage_command || '');
          setConventionsFile(res.worktree_strategy.conventions_file || '');
          setHarnesses(res.worktree_strategy.harnesses || {});
          setPrTemplate(res.worktree_strategy.pr_template || '');
          setConflictPolicy(res.conflict_policy);
          setFeatureLifecycle(res.feature_lifecycle);
          setDefaultAgentKind(res.default_agent_kind || '');
          setDefaultModel(res.default_model || '');
          setDefaultLoopIterations(res.default_loop_iterations != null ? String(res.default_loop_iterations) : '');
          setArtifactSubdir(res.artifact_subdir || 'artifacts/');
          setCommitArtifacts(Boolean(res.commit_artifacts));
        }
        const reposRes = await invoke<any[]>('get_repositories_for_project', { projectId: activeProject.id });
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
      const list = await invoke<ProjectMemoryEntry[]>('project_memory_list', { projectId: activeProject.id });
      setMemories(list ?? []);
    } catch (err) { setMemError(formatError(err)); }
    finally { setIsMemoriesLoading(false); }
  };

  const handleSaveMemory = async (e: React.FormEvent) => {
    e.preventDefault();
    const key = newMemKey.trim(); const value = newMemVal.trim();
    if (!key || !value) { setMemError('Key and Value cannot be empty.'); return; }
    try {
      await invoke('project_memory_upsert', { id: editingMemory ? editingMemory.id : null, projectId: activeProject.id, key, value, source: editingMemory ? editingMemory.source : 'human' });
      setNewMemKey(''); setNewMemVal(''); setEditingMemory(null);
      fetchMemories();
    } catch (err) { setMemError(formatError(err)); }
  };

  const handleDeleteMemory = async (id: string) => {
    try { await invoke('project_memory_delete', { id }); fetchMemories(); }
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
          const repos = await invoke<string[]>('fetch_provider_repos', { providerId: p.id });
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
    try { await invoke('test_machine_connection', { machineId: remoteHost }); setConnectionStatus('success'); }
    catch (err) { setConnectionStatus('error'); setErrorMsg('Connection test failed: ' + formatError(err)); setStatus('error'); }
    finally { setIsTestingConnection(false); }
  };

  const checkDirtyRepositories = async (repos: AvailableRepo[]): Promise<RepoDirtyStatus[]> => {
    if (repos.length === 0) return [];
    try {
      const res = await invoke<RepoDirtyStatus[]>('check_repos_dirty', { projectId: activeProject.id, repoPaths: repos.map(r => r.path) });
      return res.filter(r => r.has_uncommitted || r.has_unpushed);
    } catch (err) { reportError(err, { kind: 'internal' }); return []; }
  };

  const saveAllSettings = async () => {
    const machineId = computeType === 'remote' ? remoteHost : 'local';
    if (machineId) {
      try { await invoke('set_agent_configs', { machineId, agents: agentConfigs.filter(a => a.kind !== 'antigravity').map(a => ({ kind: a.kind, enabled: a.enabled })) }); }
      catch (err) { reportError(err, { kind: 'validation' }); }
    }
    await invoke('update_project', { id: activeProject.id, config: { name: projectName, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null, repos: selectedRepos.map(r => ({ repo_path: r.path, provider_id: r.providerId })) } });
    await saveProjectSettings(activeProject.id, { default_branch: defaultBranch, branch_prefix: branchPrefix, test_command: testCommand || null, build_command: buildCommand || null, coverage_command: coverageCommand || null, conventions_file: conventionsFile || null, pr_template: prTemplate || null, harnesses: Object.keys(harnesses).length > 0 ? harnesses : null, conflict_policy: conflictPolicy, feature_lifecycle: featureLifecycle, default_agent_kind: defaultAgentKind || null, default_model: defaultModel || null, default_loop_iterations: defaultLoopIterations.trim() ? parseInt(defaultLoopIterations, 10) : null, artifact_subdir: artifactSubdir || 'artifacts/', commit_artifacts: commitArtifacts });
  };

  const handleSave = async () => {
    setStatus('saving'); setErrorMsg('');
    const reposChanged = selectedRepos.length !== originalRepos.length || selectedRepos.some(r => !originalRepos.some(o => o.path === r.path));
    const computeChanged = computeType !== activeProject.compute_type || remoteHost !== activeProject.remote_host;
    const isCurrentlyFailedOrBootstrapping = activeProject.status === 'error' || activeProject.status === 'bootstrapping';
    const machineId = computeType === 'remote' ? remoteHost : 'local';
    if (machineId) {
      try { await invoke('set_agent_configs', { machineId, agents: agentConfigs.filter(a => a.kind !== 'antigravity').map(a => ({ kind: a.kind, enabled: a.enabled })) }); }
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
        await invoke('update_project', { id: activeProject.id, config: { name: projectName, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null, repos: selectedRepos.map(r => ({ repo_path: r.path, provider_id: r.providerId })) } });
        await saveProjectSettings(activeProject.id, { default_branch: defaultBranch, branch_prefix: branchPrefix, test_command: testCommand || null, build_command: buildCommand || null, coverage_command: coverageCommand || null, conventions_file: conventionsFile || null, pr_template: prTemplate || null, harnesses: Object.keys(harnesses).length > 0 ? harnesses : null, conflict_policy: conflictPolicy, feature_lifecycle: featureLifecycle, default_agent_kind: defaultAgentKind || null, default_model: defaultModel || null, default_loop_iterations: defaultLoopIterations.trim() ? parseInt(defaultLoopIterations, 10) : null, artifact_subdir: artifactSubdir || 'artifacts/', commit_artifacts: commitArtifacts });
        setProjects(prev => prev.map(p => p.id === activeProject.id ? { ...p, name: projectName, repos: selectedRepos.length, nodes: computeType === 'local' ? 4 : 8 } : p));
        setStatus('success'); setOriginalRepos(selectedRepos);
        setTimeout(() => setStatus('idle'), 1500);
      } catch (err) { setStatus('error'); setErrorMsg(formatError(err)); }
    }
  };

  const proceedWithReBootstrap = async () => {
    setBootstrapStep('bootstrapping'); setBootstrapError('');
    try {
      const existing = await invoke<ProjectSettingsData | null>('get_proposed_strategy', { projectId: activeProject.id });
      await invoke('update_project', { id: activeProject.id, config: { name: projectName, compute_type: computeType, remote_host: computeType === 'remote' ? remoteHost : null, repos: selectedRepos.map(r => ({ repo_path: r.path, provider_id: r.providerId })) } });
      const strategy = await invoke<WorktreeStrategy>('bootstrap_project', { projectId: activeProject.id });
      const ext = existing?.worktree_strategy;
      setDefaultBranch(ext?.default_branch ?? strategy.default_branch);
      setBranchPrefix(ext?.branch_prefix ?? strategy.branch_prefix);
      setTestCommand(ext?.test_command ?? strategy.test_command ?? '');
      setPrTemplate(ext?.pr_template ?? strategy.pr_template ?? '');
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
      await invoke('delete_project', { id: activeProject.id });
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
    harnesses, setHarnesses, prTemplate, setPrTemplate, conflictPolicy, setConflictPolicy,
    featureLifecycle, setFeatureLifecycle, defaultAgentKind, setDefaultAgentKind, defaultModel, setDefaultModel,
    defaultLoopIterations, setDefaultLoopIterations, availableModelsForDefault, isLoadingModelsForDefault,
    agentConfigs, setAgentConfigs, isRefreshingAgents, artifactSubdir, setArtifactSubdir, commitArtifacts, setCommitArtifacts,
    dirtyWarningRepos, setDirtyWarningRepos, pendingActionAfterConfirm, setPendingActionAfterConfirm, showDeleteConfirm, setShowDeleteConfirm,
    workflows, overrides, setOverrides, isLoadingOverrides, overridesError, expandedWf, setExpandedWf,
    rowModels, rowModelsLoading, savedPulse, overrideAgentKinds, overridesMachineId,
    handleSave, handleDeleteClick, proceedWithReBootstrap, proceedWithDelete, handleApproveStrategy,
    handleSaveMemory, handleDeleteMemory, handleEditMemoryClick, handleCancelEdit,
    fetchAllReposFromProviders, toggleRepo, handleTestConnection, fetchWorkspaceHealth, fetchAgentConfigs,
    toggleWorkflowExpanded, handleAgentChange, handleModelChange, handleClearRow, workflowOverrideCount,
    inheritedAgent, inheritedModel, effectiveAgentForRow,
  };

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
