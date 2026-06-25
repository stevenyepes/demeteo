import React, { useState, useEffect } from 'react';
import {
    Settings, Save, Check, RotateCw, GitBranch, ShieldAlert,
    Trash2, Box, Search, Plus, X, AlertTriangle, HardDrive, Server, Globe,
    Activity, RefreshCw, ChevronDown, ChevronUp, Zap, CircleAlert, Brain, Edit,
    FileText, Cpu, Workflow as WorkflowIcon, RotateCcw
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { ConfigOptionValue, ProjectMemoryEntry, WorkflowOverride, StepConfig } from '../types';
import { getAgentModels } from '../lib/agentModels';
import { formatError } from '../lib/errors';
import { useErrorBus } from '../lib/errorBus';

interface Project {
    id: string;
    name: string;
    status: string;
    repos: number;
    nodes: number;
    spend: number;
    tokens: number;
    compute_type?: string;
    remote_host?: string | null;
}

interface Machine {
    id: string;
    name: string;
    host: string;
    port: number;
    username: string;
    auth_type: string;
    key_path?: string | null;
    agents?: string | null;
    use_login_shell?: boolean | null;
    setup_commands?: string | null;
}

interface Provider {
    id: string;
    type: string;
    name: string;
    host: string;
    pat: string;
    username: string;
    avatarUrl: string;
}

interface AvailableRepo {
    path: string;
    providerId: string;
}

interface ProjectSettingsProps {
    setView: (view: string) => void;
    activeProject: Project;
    setProjects: React.Dispatch<React.SetStateAction<Project[]>>;
    setCurrentProject: (id: string | null) => void;
    providers: Provider[];
}

interface WorktreeStrategy {
    default_branch: string;
    branch_prefix: string;
    test_command: string | null;
    pr_template: string | null;
    harnesses?: { [key: string]: string } | null;
}

interface ProjectSettings {
    project_id: string;
    worktree_strategy: WorktreeStrategy;
    conflict_policy: string;
    feature_lifecycle: string;
    default_agent_kind?: string | null;
    default_model?: string | null;
    /**
     * Default `on_failure` retry-loop budget for this project. `null` =
     * use the engine default (3). Overridable per run. Mirrors
     * `ProjectSettings::default_loop_iterations` in Rust.
     */
    default_loop_iterations?: number | null;
    /**
     * Repo-relative folder where agents write their reports
     * (`research-report.md`, `critic-review.md`, …). Default `artifacts/`.
     * Mirrors `ProjectSettings::artifact_subdir` in Rust.
     */
    artifact_subdir?: string;
    /**
     * When true, the orchestrator commits the artifact folder into the
     * feature branch (legacy behaviour). When false (default), reports
     * stay in demeteo's local store + UI only. Mirrors
     * `ProjectSettings::commit_artifacts` in Rust.
     */
    commit_artifacts?: boolean;
}

interface RepoDirtyStatus {
    repo_path: string;
    has_uncommitted: boolean;
    has_unpushed: boolean;
}

interface WorktreeInfo {
    path: string;
    branch: string | null;
    is_locked: boolean;
}

interface RepoHealthStatus {
    repo_path: string;
    is_cloned: boolean;
    head_branch: string | null;
    worktrees: WorktreeInfo[];
    has_uncommitted: boolean;
    has_unpushed: boolean;
}

interface AgentConfigView {
    kind: string;
    enabled: boolean;
    available: boolean;
    install_command: string;
}


export default function ProjectSettingsView({ 
    setView, 
    activeProject, 
    setProjects, 
    setCurrentProject,
    providers 
}: ProjectSettingsProps) {
    const [isLoading, setIsLoading] = useState(true);
    const [activeTab, setActiveTab] = useState<'general' | 'strategy' | 'overrides' | 'memory'>('general');
    const [status, setStatus] = useState<'idle' | 'saving' | 'success' | 'error'>('idle');
    const [errorMsg, setErrorMsg] = useState('');
    const { reportError } = useErrorBus();

    // Project Memory states
    const [memories, setMemories] = useState<ProjectMemoryEntry[]>([]);
    const [isMemoriesLoading, setIsMemoriesLoading] = useState(false);
    const [editingMemory, setEditingMemory] = useState<ProjectMemoryEntry | null>(null);
    const [newMemKey, setNewMemKey] = useState('');
    const [newMemVal, setNewMemVal] = useState('');
    const [memError, setMemError] = useState('');

    // General Configuration States
    const [projectName, setProjectName] = useState(activeProject.name);
    const [computeType, setComputeType] = useState(activeProject.compute_type || 'local');
    const [remoteHost, setRemoteHost] = useState(activeProject.remote_host || '');
    const [machines, setMachines] = useState<Machine[]>([]);
    const [isTestingConnection, setIsTestingConnection] = useState(false);
    const [connectionStatus, setConnectionStatus] = useState<'idle' | 'success' | 'error'>('idle');

    // Repository States
    const [selectedRepos, setSelectedRepos] = useState<AvailableRepo[]>([]);
    const [originalRepos, setOriginalRepos] = useState<AvailableRepo[]>([]);
    const [isRepoModalOpen, setIsRepoModalOpen] = useState(false);
    const [repoSearch, setRepoSearch] = useState('');
    const [availableRepos, setAvailableRepos] = useState<AvailableRepo[]>([]);
    const [isLoadingRepos, setIsLoadingRepos] = useState(false);

    // Re-bootstrap flow states
    const [bootstrapStep, setBootstrapStep] = useState<'form' | 'bootstrapping' | 'strategy_proposal' | 'bootstrap_success' | 'error'>('form');
    const [bootstrapError, setBootstrapError] = useState('');

    // Workspace Health panel states
    const [healthData, setHealthData] = useState<RepoHealthStatus[] | null>(null);
    const [isLoadingHealth, setIsLoadingHealth] = useState(false);
    const [healthExpanded, setHealthExpanded] = useState(true);
    const [showHealthPanel, setShowHealthPanel] = useState(false);
    const [healthError, setHealthError] = useState('');

    // Strategy Form States
    const [defaultBranch, setDefaultBranch] = useState('main');
    const [branchPrefix, setBranchPrefix] = useState('demeteo/features/');
    const [testCommand, setTestCommand] = useState('');
    const [harnesses, setHarnesses] = useState<{ [key: string]: string }>({});
    const [prTemplate, setPrTemplate] = useState('');
    const [conflictPolicy, setConflictPolicy] = useState('always_gate');
    const [featureLifecycle, setFeatureLifecycle] = useState('archive');

    // Warning Modals
    const [dirtyWarningRepos, setDirtyWarningRepos] = useState<RepoDirtyStatus[]>([]);
    const [pendingActionAfterConfirm, setPendingActionAfterConfirm] = useState<'save' | 'delete' | null>(null);
    const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

    // Coding Agent configs
    const [agentConfigs, setAgentConfigs] = useState<AgentConfigView[]>([]);
    const [isRefreshingAgents, setIsRefreshingAgents] = useState(false);

    // Default AI Executor States
    const [defaultAgentKind, setDefaultAgentKind] = useState<string>('');
    const [defaultModel, setDefaultModel] = useState<string>('');
    // Empty string = use the engine default (3). Stored as a string for the
    // input; converted to number|null on save.
    const [defaultLoopIterations, setDefaultLoopIterations] = useState<string>('');
    const [availableModelsForDefault, setAvailableModelsForDefault] = useState<ConfigOptionValue[]>([]);
    const [isLoadingModelsForDefault, setIsLoadingModelsForDefault] = useState(false);

    // Project-scoped harness/model overrides (migrations V14/V15). Workflows
    // are global; this tab pins a coding agent + model for a workflow — or a
    // single step within it — *for this project*. Project Settings has the
    // compute-machine context, so we can offer a probed model dropdown here,
    // something the WorkflowEditor can't.
    //
    // Everything is keyed by `ovKey(workflowId, stepId)` where stepId === ''
    // is the workflow-level row (applies to every step). This mirrors the Rust
    // model where `step_id = None` is persisted as ''.
    const WF_LEVEL = '';
    const ovKey = (workflowId: string, stepId: string) => `${workflowId}::${stepId}`;

    const [workflows, setWorkflows] = useState<{ id: string; name: string; description: string; steps: StepConfig[] }[]>([]);
    const [overrides, setOverrides] = useState<Record<string, { agent_kind: string | null; model: string | null }>>({});
    const [isLoadingOverrides, setIsLoadingOverrides] = useState(false);
    const [overridesError, setOverridesError] = useState('');
    const [expandedWf, setExpandedWf] = useState<Record<string, boolean>>({});
    // Probed model lists keyed by ovKey (each row's effective agent may differ).
    const [rowModels, setRowModels] = useState<Record<string, ConfigOptionValue[]>>({});
    const [rowModelsLoading, setRowModelsLoading] = useState<Record<string, boolean>>({});
    // Transient per-row "Saved" pulse after an immediate persist.
    const [savedPulse, setSavedPulse] = useState<Record<string, boolean>>({});

    const overridesMachineId = computeType === 'remote' ? remoteHost : 'local';
    // The harness choices mirror the agents actually installed *and* enabled on
    // the target machine, so a user can't pin a workflow to a harness that's
    // missing (CLI binary not installed) or that they've disabled for this
    // workspace. `available` is the probed install status; `enabled` is the
    // user's per-workspace curation — a harness must satisfy both.
    const overrideAgentKinds = agentConfigs
        .filter(a => a.enabled && a.available && a.kind !== 'antigravity')
        .map(a => a.kind);

    // Effective agent a step inherits when it carries no step-level override:
    // workflow-level override → workflow author's step agent → project default.
    const inheritedAgent = (workflowId: string, step: StepConfig): string => {
        const wfOv = overrides[ovKey(workflowId, WF_LEVEL)];
        return wfOv?.agent_kind || step.agent_kind || defaultAgentKind || '';
    };
    const inheritedModel = (workflowId: string, step: StepConfig): string => {
        const wfOv = overrides[ovKey(workflowId, WF_LEVEL)];
        return wfOv?.model || step.model || defaultModel || '';
    };
    // The agent a row's model dropdown should be probed against: its own agent
    // override if set, else whatever it inherits.
    const effectiveAgentForRow = (workflowId: string, step: StepConfig | null): string => {
        if (step === null) {
            return overrides[ovKey(workflowId, WF_LEVEL)]?.agent_kind || defaultAgentKind || '';
        }
        return overrides[ovKey(workflowId, step.id)]?.agent_kind || inheritedAgent(workflowId, step);
    };

    const probeModels = async (key: string, agentKind: string) => {
        if (!agentKind) {
            setRowModels(prev => ({ ...prev, [key]: [] }));
            return;
        }
        setRowModelsLoading(prev => ({ ...prev, [key]: true }));
        try {
            const models = await getAgentModels(overridesMachineId, agentKind);
            setRowModels(prev => ({ ...prev, [key]: models }));
        } catch (err) {
            console.warn('Failed to probe models for', key, agentKind, err);
            setRowModels(prev => ({ ...prev, [key]: [] }));
        } finally {
            setRowModelsLoading(prev => ({ ...prev, [key]: false }));
        }
    };

    const loadWorkflowOverrides = async () => {
        setIsLoadingOverrides(true);
        setOverridesError('');
        try {
            const [wfList, ovList] = await Promise.all([
                invoke<{ id: string; name: string; description: string; steps: StepConfig[] }[]>('workflow_list'),
                invoke<WorkflowOverride[]>('get_workflow_overrides', { projectId: activeProject.id }),
            ]);
            setWorkflows(wfList.map(w => ({ id: w.id, name: w.name, description: w.description, steps: w.steps ?? [] })));
            const map: Record<string, { agent_kind: string | null; model: string | null }> = {};
            for (const ov of ovList) {
                map[ovKey(ov.workflow_id, ov.step_id ?? WF_LEVEL)] = {
                    agent_kind: ov.agent_kind ?? null,
                    model: ov.model ?? null,
                };
            }
            setOverrides(map);
            // Auto-expand workflows that already carry any override so the user
            // immediately sees what's customized.
            const toExpand: Record<string, boolean> = {};
            for (const ov of ovList) toExpand[ov.workflow_id] = true;
            setExpandedWf(toExpand);
            // Warm the model dropdowns for rows that pin an agent.
            for (const ov of ovList) {
                if (ov.agent_kind) probeModels(ovKey(ov.workflow_id, ov.step_id ?? WF_LEVEL), ov.agent_kind);
            }
        } catch (err) {
            console.error('Failed to load workflow overrides:', err);
            setOverridesError(formatError(err));
        } finally {
            setIsLoadingOverrides(false);
        }
    };

    useEffect(() => {
        if (activeTab === 'overrides') {
            loadWorkflowOverrides();
        }
    }, [activeTab, activeProject.id, overridesMachineId]);

    // Expanding a workflow lazily probes models for every row in it (workflow
    // default + each non-gate step), using each row's effective agent. The
    // getAgentModels cache dedupes repeated (machine, agent) probes.
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

    // Persist one row immediately. stepId === '' is the workflow-level row; the
    // backend treats null agent+model as "clear" and deletes the row.
    const persistOverride = async (
        workflowId: string,
        stepId: string,
        next: { agent_kind: string | null; model: string | null },
    ) => {
        const key = ovKey(workflowId, stepId);
        try {
            await invoke('set_workflow_override', {
                projectId: activeProject.id,
                workflowId,
                stepId: stepId || null,
                agentKind: next.agent_kind,
                model: next.model,
            });
            setSavedPulse(prev => ({ ...prev, [key]: true }));
            setTimeout(() => setSavedPulse(prev => ({ ...prev, [key]: false })), 1400);
        } catch (err) {
            console.error('Failed to save workflow override:', err);
            setOverridesError(formatError(err));
        }
    };

    const handleAgentChange = (workflowId: string, stepId: string, step: StepConfig | null, agentKind: string) => {
        // Changing the harness invalidates the probed model list, so clear the
        // model selection and re-probe for the new effective agent.
        const next = { agent_kind: agentKind || null, model: null };
        const key = ovKey(workflowId, stepId);
        setOverrides(prev => ({ ...prev, [key]: next }));
        // When cleared, fall back to the inherited agent for probing.
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

    // Does a workflow have any active override (workflow-level or any step)?
    const workflowOverrideCount = (wf: { id: string; steps: StepConfig[] }): number => {
        let n = 0;
        if (overrides[ovKey(wf.id, WF_LEVEL)]?.agent_kind || overrides[ovKey(wf.id, WF_LEVEL)]?.model) n++;
        for (const s of wf.steps) {
            const o = overrides[ovKey(wf.id, s.id)];
            if (o?.agent_kind || o?.model) n++;
        }
        return n;
    };

    // The harness + model + reset controls for a single row. `step === null`
    // renders the workflow-level row (whose empty option = the project
    // default); a step row's empty option = "Inherit", and shows what the step
    // would resolve to so the user always sees the effective harness/model.
    const renderOverrideRow = (wf: { id: string; steps: StepConfig[] }, step: StepConfig | null) => {
        const stepId = step ? step.id : WF_LEVEL;
        const key = ovKey(wf.id, stepId);
        const ov = overrides[key] ?? { agent_kind: null, model: null };
        const models = rowModels[key] ?? [];
        const modelsLoading = Boolean(rowModelsLoading[key]);
        const effectiveAgent = effectiveAgentForRow(wf.id, step);
        const rowActive = Boolean(ov.agent_kind || ov.model);

        const inhA = step ? inheritedAgent(wf.id, step) : (defaultAgentKind || '');
        const inhM = step ? inheritedModel(wf.id, step) : (defaultModel || '');
        const agentPlaceholder = step
            ? `Inherit${inhA ? ` · ${inhA.replace(/-/g, ' ')}` : ' · built-in'}`
            : 'Project default';
        const modelEnabled = Boolean(effectiveAgent);
        const modelPlaceholder = !modelEnabled
            ? 'Pick a harness first'
            : inhM
                ? `Inherit · ${inhM}`
                : 'Agent default model';

        return (
            <div className="grid grid-cols-1 sm:grid-cols-[1fr_1fr_auto] gap-3 items-end">
                <div>
                    <label className="flex items-center gap-1.5 text-[10px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                        <Cpu className="w-3 h-3" /> Harness
                    </label>
                    <select
                        value={ov.agent_kind ?? ''}
                        onChange={e => handleAgentChange(wf.id, stepId, step, e.target.value)}
                        className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-violet-500/50 capitalize"
                    >
                        <option value="">{agentPlaceholder}</option>
                        {overrideAgentKinds.map(k => (
                            <option key={k} value={k}>{k.replace(/-/g, ' ')}</option>
                        ))}
                        {ov.agent_kind && !overrideAgentKinds.includes(ov.agent_kind) && (
                            <option value={ov.agent_kind}>{ov.agent_kind.replace(/-/g, ' ')} (unavailable)</option>
                        )}
                    </select>
                </div>
                <div>
                    <label className="flex items-center gap-1.5 text-[10px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                        <Zap className="w-3 h-3" /> Model
                    </label>
                    {modelsLoading ? (
                        <div className="w-full bg-[#08090c]/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-400 flex items-center gap-2">
                            <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                            <span>Probing models…</span>
                        </div>
                    ) : (
                        <select
                            value={ov.model ?? ''}
                            onChange={e => handleModelChange(wf.id, stepId, e.target.value)}
                            disabled={!modelEnabled}
                            className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-violet-500/50 disabled:opacity-40 disabled:cursor-not-allowed"
                        >
                            <option value="">{modelPlaceholder}</option>
                            {models.map(m => (
                                <option key={m.value} value={m.value}>{m.name}</option>
                            ))}
                            {ov.model && !models.some(m => m.value === ov.model) && (
                                <option value={ov.model}>{ov.model} (custom)</option>
                            )}
                        </select>
                    )}
                </div>
                <div className="flex items-center gap-2 pb-0.5">
                    {savedPulse[key] && (
                        <span className="flex items-center gap-1 text-[10px] text-emerald-400 font-medium shrink-0 animate-fadeIn">
                            <Check className="w-3 h-3" /> Saved
                        </span>
                    )}
                    <button
                        type="button"
                        onClick={() => handleClearRow(wf.id, stepId)}
                        disabled={!rowActive}
                        title="Reset to inherited"
                        className="p-2 rounded-lg text-slate-500 hover:text-white bg-white/5 border border-white/10 hover:bg-white/10 transition-all disabled:opacity-25 disabled:cursor-not-allowed shrink-0"
                    >
                        <RotateCcw className="w-3.5 h-3.5" />
                    </button>
                </div>
            </div>
        );
    };

    // Artifact handling (migration V12). The orchestrator writes the
    // reports each step produces into the worktree under
    // `artifactSubdir` (default `artifacts/`). The
    // `commitArtifacts` flag controls whether those files end up in
    // the feature branch's commit history or stay only in demeteo's
    // local `FsArtifactStore`. Both fields round-trip through
    // `ProjectSettings` so the user can edit them here.
    const [artifactSubdir, setArtifactSubdir] = useState<string>('artifacts/');
    const [commitArtifacts, setCommitArtifacts] = useState<boolean>(false);

    useEffect(() => {
        let cancelled = false;
        const fetchModels = async () => {
            if (!defaultAgentKind) {
                setAvailableModelsForDefault([]);
                return;
            }
            setIsLoadingModelsForDefault(true);
            try {
                const machineId = computeType === 'remote' ? remoteHost : 'local';
                const models = await getAgentModels(machineId, defaultAgentKind);
                if (!cancelled) setAvailableModelsForDefault(models);
            } catch (err) {
                if (!cancelled) {
                    console.warn("Failed to fetch models for agent:", defaultAgentKind, err);
                    setAvailableModelsForDefault([]);
                }
            } finally {
                if (!cancelled) setIsLoadingModelsForDefault(false);
            }
        };
        fetchModels();
        return () => { cancelled = true; };
    }, [defaultAgentKind, computeType, remoteHost]);

    useEffect(() => {
        setConnectionStatus('idle');
    }, [remoteHost]);

    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const list: Machine[] = await invoke('get_machines');
                if (!cancelled) setMachines(list ?? []);
            } catch (err) {
                // The settings page can't render without the machine list.
                reportError(err, { kind: "internal" });
            }
        })();
        return () => { cancelled = true; };
    }, []);

    const fetchAgentConfigs = async (refresh: boolean = false) => {
        const machineId = computeType === 'remote' ? remoteHost : 'local';
        if (computeType === 'remote' && !remoteHost) {
            setAgentConfigs([]);
            return;
        }
        if (refresh) setIsRefreshingAgents(true);
        try {
            const configs = await invoke<AgentConfigView[]>('get_agent_configs', {
                machineId,
                refresh,
            });
            setAgentConfigs(configs);
        } catch (err) {
            // The backend has no rows for this machine — that's a
            // legitimate state, not an error. Show an empty list and
            // let the user save new configs. Previously this catch
            // block silently replaced the error with mock data that
            // hardcoded `available: true`, which let the user start a
            // feature against an agent that was never installed.
            const message = formatError(err);
            console.warn("No agent configs found for machine:", machineId, "—", message);
            setAgentConfigs([]);
        } finally {
            if (refresh) setIsRefreshingAgents(false);
        }
    };

    useEffect(() => {
        fetchAgentConfigs();
    }, [computeType, remoteHost]);


    const fetchWorkspaceHealth = async () => {
        setIsLoadingHealth(true);
        setHealthError('');
        try {
            const data = await invoke<RepoHealthStatus[]>('get_workspace_health', {
                projectId: activeProject.id
            });
            setHealthData(data);
            setShowHealthPanel(true);
            setHealthExpanded(true);
        } catch (err) {
            // DO NOT paper over backend errors with fake "is_cloned: true"
            // data — that's how the user ends up with a panel that says
            // HEALTHY while the remote has no .demeteo directory at all.
            // Surface the error, leave the panel closed, and let the
            // user re-trigger the bootstrap from the settings screen.
            const message = formatError(err);
            console.error('Failed to fetch workspace health:', err);
            setHealthError(message);
            setHealthData([]);
            setShowHealthPanel(false);
        } finally {
            setIsLoadingHealth(false);
        }
    };

    useEffect(() => {
        const fetchSettingsAndRepos = async () => {
            setIsLoading(true);
            try {
                // Fetch project settings/strategy
                const res = await invoke<ProjectSettings | null>('get_proposed_strategy', {
                    projectId: activeProject.id
                });
                if (res) {
                    setDefaultBranch(res.worktree_strategy.default_branch);
                    setBranchPrefix(res.worktree_strategy.branch_prefix);
                    setTestCommand(res.worktree_strategy.test_command || '');
                    setHarnesses(res.worktree_strategy.harnesses || {});
                    setPrTemplate(res.worktree_strategy.pr_template || '');
                    setConflictPolicy(res.conflict_policy);
                    setFeatureLifecycle(res.feature_lifecycle);
                    setDefaultAgentKind(res.default_agent_kind || '');
                    setDefaultModel(res.default_model || '');
                    setDefaultLoopIterations(
                        res.default_loop_iterations != null ? String(res.default_loop_iterations) : ''
                    );
                    setArtifactSubdir(res.artifact_subdir || 'artifacts/');
                    setCommitArtifacts(Boolean(res.commit_artifacts));
                }

                // Fetch configured repos
                const reposRes = await invoke<any[]>('get_repositories_for_project', {
                    projectId: activeProject.id
                });
                const mappedRepos = reposRes.map(r => ({
                    path: r.repo_path,
                    providerId: r.provider_id
                }));
                setSelectedRepos(mappedRepos);
                setOriginalRepos(mappedRepos);
            } catch (err) {
                // Surface the error to the user instead of silently
                // filling the form with fake defaults — that's how
                // the workspace-health panel ended up claiming a
                // non-existent clone was healthy. The user can see
                // the red status badge at the top of the settings
                // and re-trigger the bootstrap.
                const message = formatError(err);
                console.error("Failed to load project configuration details:", err);
                setErrorMsg(message);
                setStatus('error');
                setSelectedRepos([]);
                setOriginalRepos([]);
            } finally {
                setIsLoading(false);
            }
        };
        fetchSettingsAndRepos();

        // Auto-load health panel if workspace is already idle (already bootstrapped)
        if (activeProject.status === 'idle') {
            setShowHealthPanel(true);
            fetchWorkspaceHealth();
        }
    }, [activeProject.id]);

    const fetchMemories = async () => {
        setIsMemoriesLoading(true);
        setMemError('');
        try {
            const list = await invoke<ProjectMemoryEntry[]>('project_memory_list', {
                projectId: activeProject.id
            });
            setMemories(list ?? []);
        } catch (err: any) {
            console.error("Failed to load project memory:", err);
            setMemError(formatError(err));
        } finally {
            setIsMemoriesLoading(false);
        }
    };

    useEffect(() => {
        if (activeTab === 'memory') {
            fetchMemories();
        }
    }, [activeTab, activeProject.id]);

    const handleSaveMemory = async (e: React.FormEvent) => {
        e.preventDefault();
        const key = newMemKey.trim();
        const value = newMemVal.trim();
        if (!key || !value) {
            setMemError("Key and Value cannot be empty.");
            return;
        }

        try {
            await invoke('project_memory_upsert', {
                id: editingMemory ? editingMemory.id : null,
                projectId: activeProject.id,
                key,
                value,
                source: editingMemory ? editingMemory.source : 'human'
            });
            setNewMemKey('');
            setNewMemVal('');
            setEditingMemory(null);
            fetchMemories();
        } catch (err: any) {
            console.error("Failed to save memory:", err);
            setMemError(formatError(err));
        }
    };

    const handleDeleteMemory = async (id: string) => {
        try {
            await invoke('project_memory_delete', { id });
            fetchMemories();
        } catch (err: any) {
            console.error("Failed to delete memory:", err);
            setMemError(formatError(err));
        }
    };

    const handleEditMemoryClick = (entry: ProjectMemoryEntry) => {
        setEditingMemory(entry);
        setNewMemKey(entry.key);
        setNewMemVal(entry.value);
    };

    const handleCancelEdit = () => {
        setEditingMemory(null);
        setNewMemKey('');
        setNewMemVal('');
    };

    const fetchAllReposFromProviders = async () => {
        if (providers.length === 0) return;
        setIsLoadingRepos(true);
        try {
            const allRepos = await Promise.all(providers.map(async (p) => {
                try {
                    const repos = await invoke<string[]>('fetch_provider_repos', {
                        providerId: p.id
                    });
                    return repos.map(r => ({ path: r, providerId: p.id }));
                } catch (err) {
                    // One provider's repos failed — others may succeed, so
                    // keep the partial result and surface the failure.
                    reportError(err, { kind: "provider" });
                    return [];
                }
            }));
            const flatRepos = allRepos.flat();
            // Deduplicate by path
            const uniqueRepos: AvailableRepo[] = [];
            const seen = new Set<string>();
            for (const r of flatRepos) {
                if (!seen.has(r.path)) {
                    seen.add(r.path);
                    uniqueRepos.push(r);
                }
            }
            setAvailableRepos(uniqueRepos);
        } catch (err) {
            // Don't fill the repo picker with fake "acme-corp/*"
            // mock entries — that lets the user select repos that
            // don't exist on any provider and silently breaks the
            // bootstrap. Show an empty list and let the user retry.
            const message = formatError(err);
            console.error("Failed to fetch repositories from providers:", err);
            setErrorMsg(message);
            setStatus('error');
            setAvailableRepos([]);
        } finally {
            setIsLoadingRepos(false);
        }
    };

    const toggleRepo = (repo: AvailableRepo) => {
        setSelectedRepos(prev =>
            prev.some(r => r.path === repo.path)
                ? prev.filter(r => r.path !== repo.path)
                : [...prev, repo]
        );
    };

    const handleTestConnection = async () => {
        if (!remoteHost) return;
        setIsTestingConnection(true);
        setConnectionStatus('idle');
        try {
            await invoke('test_machine_connection', { machineId: remoteHost });
            setConnectionStatus('success');
        } catch (err) {
            console.error("Connection check failed:", err);
            setConnectionStatus('error');
            setErrorMsg("Connection test failed: " + formatError(err));
            setStatus('error');
        } finally {
            setIsTestingConnection(false);
        }
    };

    const checkDirtyRepositories = async (reposToCheck: AvailableRepo[]): Promise<RepoDirtyStatus[]> => {
        if (reposToCheck.length === 0) return [];
        try {
            const res = await invoke<RepoDirtyStatus[]>('check_repos_dirty', {
                projectId: activeProject.id,
                repoPaths: reposToCheck.map(r => r.path)
            });
            return res.filter(r => r.has_uncommitted || r.has_unpushed);
        } catch (err) {
            // The dirty-status check is best-effort; surface the failure
            // so the user knows the warnings may be incomplete.
            reportError(err, { kind: "internal" });
            return [];
        }
    };

    const handleSave = async () => {
        setStatus('saving');
        setErrorMsg('');

        // Determine if repositories or compute type changed
        const reposChanged = selectedRepos.length !== originalRepos.length || 
            selectedRepos.some(r => !originalRepos.some(o => o.path === r.path));
        const computeChanged = computeType !== activeProject.compute_type || remoteHost !== activeProject.remote_host;
        const isCurrentlyFailedOrBootstrapping = activeProject.status === 'error' || activeProject.status === 'bootstrapping';

        try {
            const machineId = computeType === 'remote' ? remoteHost : 'local';
            if (machineId) {
                await invoke('set_agent_configs', {
                    machineId,
                    agents: agentConfigs.filter(a => a.kind !== 'antigravity').map(a => ({ kind: a.kind, enabled: a.enabled }))
                });
            }
        } catch (err) {
            // User-initiated save failed — surface the error so the user
            // knows the configuration was not persisted.
            reportError(err, { kind: "validation" });
        }

        if (reposChanged || computeChanged || isCurrentlyFailedOrBootstrapping) {
            // Check if any repositories are being removed
            const removedRepos = originalRepos.filter(o => !selectedRepos.some(s => s.path === o.path));
            if (removedRepos.length > 0) {
                const dirtyList = await checkDirtyRepositories(removedRepos);
                if (dirtyList.length > 0) {
                    setDirtyWarningRepos(dirtyList);
                    setPendingActionAfterConfirm('save');
                    setStatus('idle');
                    return; // Stop and display warning
                }
            }
            // Proceed to re-bootstrap
            await proceedWithReBootstrap();
        } else {
            // No structural changes, just save configuration and settings
            try {
                // 1. Update project configuration
                await invoke('update_project', {
                    id: activeProject.id,
                    config: {
                        name: projectName,
                        compute_type: computeType,
                        remote_host: computeType === 'remote' ? remoteHost : null,
                        repos: selectedRepos.map(r => ({
                            repo_path: r.path,
                            provider_id: r.providerId
                        }))
                    }
                });

                // 2. Save settings
                await invoke('save_project_settings', {
                    projectId: activeProject.id,
                    settings: {
                        project_id: activeProject.id,
                        worktree_strategy: {
                            default_branch: defaultBranch,
                            branch_prefix: branchPrefix,
                            test_command: testCommand || null,
                            pr_template: prTemplate || null,
                            harnesses: Object.keys(harnesses).length > 0 ? harnesses : null
                        },
                    conflict_policy: conflictPolicy,
                    feature_lifecycle: featureLifecycle,
                    default_agent_kind: defaultAgentKind || null,
                    default_model: defaultModel || null,
                    default_loop_iterations: defaultLoopIterations.trim() ? parseInt(defaultLoopIterations, 10) : null,
                    artifact_subdir: artifactSubdir || 'artifacts/',
                    commit_artifacts: commitArtifacts
                }
            });

            // 3. Update parent projects state
            setProjects(prev => prev.map(p => p.id === activeProject.id ? {
                    ...p,
                    name: projectName,
                    repos: selectedRepos.length,
                    nodes: computeType === 'local' ? 4 : 8
                } : p));

                setStatus('success');
                setOriginalRepos(selectedRepos);
                setTimeout(() => {
                    setStatus('idle');
                }, 1500);
            } catch (err: any) {
                setStatus('error');
                setErrorMsg(formatError(err));
            }
        }
    };

    const proceedWithReBootstrap = async () => {
        setBootstrapStep('bootstrapping');
        setBootstrapError('');
        try {
            // 1. Save new config structure in DB (resets status to bootstrapping)
            await invoke('update_project', {
                id: activeProject.id,
                config: {
                    name: projectName,
                    compute_type: computeType,
                    remote_host: computeType === 'remote' ? remoteHost : null,
                    repos: selectedRepos.map(r => ({
                        repo_path: r.path,
                        provider_id: r.providerId
                    }))
                }
            });

            // 2. Perform the clone & strategy detection
            const strategy = await invoke<WorktreeStrategy>('bootstrap_project', {
                projectId: activeProject.id
            });

            // 3. Set strategy proposal state
            setDefaultBranch(strategy.default_branch);
            setBranchPrefix(strategy.branch_prefix);
            setTestCommand(strategy.test_command || '');
            setHarnesses(strategy.harnesses || {});
            setPrTemplate(strategy.pr_template || '');
            setBootstrapStep('strategy_proposal');
        } catch (err: any) {
            setBootstrapStep('error');
            setBootstrapError(formatError(err));
        }
    };

    const handleApproveStrategy = async () => {
        try {
            // Save approved settings
            await invoke('save_project_settings', {
                projectId: activeProject.id,
                settings: {
                    project_id: activeProject.id,
                    worktree_strategy: {
                        default_branch: defaultBranch,
                        branch_prefix: branchPrefix,
                        test_command: testCommand || null,
                        pr_template: prTemplate || null,
                        harnesses: Object.keys(harnesses).length > 0 ? harnesses : null
                    },
                    conflict_policy: conflictPolicy,
                    feature_lifecycle: featureLifecycle,
                    default_agent_kind: defaultAgentKind || null,
                    default_model: defaultModel || null,
                    default_loop_iterations: defaultLoopIterations.trim() ? parseInt(defaultLoopIterations, 10) : null,
                    artifact_subdir: artifactSubdir || 'artifacts/',
                    commit_artifacts: commitArtifacts
                }
            });

            setProjects(prev => prev.map(p => p.id === activeProject.id ? {
                ...p,
                name: projectName,
                status: 'idle',
                repos: selectedRepos.length,
                nodes: computeType === 'local' ? 4 : 8,
                compute_type: computeType,
                remote_host: computeType === 'remote' ? remoteHost : null
            } : p));
            // Show success screen instead of silently navigating away
            setBootstrapStep('bootstrap_success');
        } catch (err: any) {
            setBootstrapStep('error');
            setBootstrapError(formatError(err));
        }
    };

    const handleDeleteClick = async () => {
        // Run dirty check on all repositories of the project before deletion
        const dirtyList = await checkDirtyRepositories(selectedRepos);
        if (dirtyList.length > 0) {
            setDirtyWarningRepos(dirtyList);
            setPendingActionAfterConfirm('delete');
        } else {
            setShowDeleteConfirm(true);
        }
    };

    const proceedWithDelete = async () => {
        setIsLoading(true);
        try {
            await invoke('delete_project', { id: activeProject.id });
            setProjects(prev => prev.filter(p => p.id !== activeProject.id));
            setCurrentProject(null);
            setView('empty-state');
        } catch (err) {
            console.error("Failed to delete workspace:", err);
            setErrorMsg("Failed to delete workspace: " + formatError(err));
            setStatus('error');
        } finally {
            setIsLoading(false);
            setShowDeleteConfirm(false);
            setDirtyWarningRepos([]);
            setPendingActionAfterConfirm(null);
        }
    };

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center p-8">
                <RotateCw className="w-8 h-8 text-cyan-400 animate-spin" />
            </div>
        );
    }

    if (bootstrapStep === 'bootstrapping') {
        return (
            <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
                <div className="absolute top-1/4 left-1/2 -translate-x-1/2 w-[600px] h-[300px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none"></div>
                <div className="glass-panel max-w-lg w-full p-8 rounded-xl flex flex-col items-center text-center relative border border-white/10 shadow-2xl">
                    <RotateCw className="w-12 h-12 text-cyan-400 animate-spin mb-6" />
                    <h2 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Re-bootstrap In Progress</h2>
                    <p className="text-sm text-slate-400 mb-6 leading-relaxed">
                        Demeteo is updating your cloned repositories and analyzing codebase strategies.
                    </p>
                    <div className="w-full bg-black/40 border border-white/5 rounded-lg p-4 font-mono text-left text-xs space-y-2.5 text-slate-300">
                        <div className="flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse"></span>
                            <span>Validating Credentials...</span>
                        </div>
                        <div className="flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse"></span>
                            <span>Syncing and Cloning Repository directories...</span>
                        </div>
                        <div className="flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-slate-600"></span>
                            <span className="text-slate-500">Pruning unconfigured repository folders...</span>
                        </div>
                    </div>
                </div>
            </div>
        );
    }

    if (bootstrapStep === 'error') {
        return (
            <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
                <div className="glass-panel max-w-lg w-full p-8 rounded-xl flex flex-col items-center text-center relative border border-ruby-500/20 shadow-2xl">
                    <AlertTriangle className="w-12 h-12 text-ruby-400 mb-4" />
                    <h2 className="text-2xl font-outfit font-bold text-white mb-2">Re-bootstrap Failed</h2>
                    <p className="text-sm text-slate-400 mb-6">
                        An error occurred while re-building the project workspace.
                    </p>
                    <div className="w-full bg-black/40 border border-ruby-500/10 rounded-lg p-4 font-mono text-left text-xs text-ruby-300 overflow-x-auto mb-6">
                        {bootstrapError}
                    </div>
                    <div className="flex gap-3">
                        <button onClick={() => setBootstrapStep('form')} className="px-5 py-2.5 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all">
                            Back to Settings
                        </button>
                        <button onClick={proceedWithReBootstrap} className="px-5 py-2.5 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg transition-all font-medium">
                            Retry Build
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    if (bootstrapStep === 'bootstrap_success') {
        const clonesCount = selectedRepos.length;
        return (
            <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
                <div className="absolute top-1/4 left-1/2 -translate-x-1/2 w-[600px] h-[300px] bg-emerald-600/10 rounded-full blur-[120px] pointer-events-none"></div>
                <div className="absolute bottom-1/4 left-1/4 w-[300px] h-[300px] bg-violet-600/10 rounded-full blur-[100px] pointer-events-none"></div>
                <div className="glass-panel max-w-lg w-full p-8 rounded-2xl flex flex-col items-center text-center relative border border-emerald-500/20 shadow-2xl">
                    {/* Success icon with ripple */}
                    <div className="relative mb-6">
                        <div className="w-20 h-20 rounded-full bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center animate-pulse">
                            <div className="w-14 h-14 rounded-full bg-emerald-500/20 border border-emerald-500/30 flex items-center justify-center">
                                <Check className="w-8 h-8 text-emerald-400 stroke-[2.5]" />
                            </div>
                        </div>
                    </div>
                    <h2 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Ready</h2>
                    <p className="text-sm text-slate-400 mb-6 leading-relaxed">
                        Bootstrap complete. {clonesCount} {clonesCount === 1 ? 'repository was' : 'repositories were'} cloned and configured.
                    </p>

                    {/* Summary chips */}
                    <div className="flex flex-wrap gap-2 justify-center mb-8">
                        <span className="flex items-center gap-1.5 px-3 py-1 rounded-full text-xs bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">
                            <Box className="w-3 h-3" /> {clonesCount} {clonesCount === 1 ? 'repo' : 'repos'} cloned
                        </span>
                        {defaultBranch && (
                            <span className="flex items-center gap-1.5 px-3 py-1 rounded-full text-xs bg-cyan-500/10 border border-cyan-500/20 text-cyan-400">
                                <GitBranch className="w-3 h-3" /> {defaultBranch}
                            </span>
                        )}
                        {testCommand && (
                            <span className="flex items-center gap-1.5 px-3 py-1 rounded-full text-xs bg-violet-500/10 border border-violet-500/20 text-violet-400">
                                <Zap className="w-3 h-3" /> {testCommand}
                            </span>
                        )}
                    </div>

                    <div className="flex gap-3 w-full">
                        <button
                            onClick={() => {
                                setBootstrapStep('form');
                                setShowHealthPanel(true);
                                fetchWorkspaceHealth();
                            }}
                            className="flex-1 px-5 py-2.5 text-sm font-medium bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all flex items-center justify-center gap-2"
                        >
                            <Activity className="w-4 h-4 text-cyan-400" /> View Workspace Health
                        </button>
                        <button
                            onClick={() => setView('home')}
                            className="flex-1 px-6 py-2.5 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg shadow-[0_0_20px_rgba(16,185,129,0.3)] transition-all flex items-center justify-center gap-2"
                        >
                            <Check className="w-4 h-4" /> Go to Project
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    if (bootstrapStep === 'strategy_proposal') {
        return (
            <div className="flex-1 overflow-y-auto p-8 relative flex items-center justify-center">
                <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[400px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none"></div>
                <div className="glass-panel max-w-xl w-full p-6 rounded-xl flex flex-col border-white/10 shadow-2xl">
                    <div className="mb-6 border-b border-white/5 pb-4">
                        <h3 className="font-outfit font-semibold text-cyan-400 uppercase tracking-widest text-xs mb-1">STRATEGY UPDATED</h3>
                        <h2 className="text-xl font-bold text-white">Approve Detected Worktree Strategy</h2>
                    </div>

                    <div className="space-y-4 max-h-[400px] overflow-y-auto pr-1">
                        <div>
                            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Branch</label>
                            <input 
                                type="text" 
                                value={defaultBranch} 
                                onChange={e => setDefaultBranch(e.target.value)}
                                className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                            />
                        </div>

                        <div>
                            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Branch Prefix</label>
                            <input 
                                type="text" 
                                value={branchPrefix} 
                                onChange={e => setBranchPrefix(e.target.value)}
                                className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                            />
                        </div>

                        <div>
                            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Test Command</label>
                            <input 
                                type="text" 
                                value={testCommand} 
                                onChange={e => setTestCommand(e.target.value)}
                                placeholder="e.g. npm test or cargo test"
                                className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                            />
                        </div>

                        <div>
                            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Conflict Resolution Policy</label>
                            <select 
                                value={conflictPolicy} 
                                onChange={e => setConflictPolicy(e.target.value)}
                                className="w-full bg-[#08090c] border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                            >
                                <option value="always_gate">Always Gate (Requires approval)</option>
                                <option value="auto_agent">Auto Agent First (Cascade to manual)</option>
                                <option value="auto_human">Immediate Manual Merge</option>
                            </select>
                        </div>

                        <div>
                            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Completed Feature Lifecycle</label>
                            <select 
                                value={featureLifecycle} 
                                onChange={e => setFeatureLifecycle(e.target.value)}
                                className="w-full bg-[#08090c] border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                            >
                                <option value="archive">Archive by default</option>
                                <option value="keep">Keep active</option>
                                <option value="auto_delete">Auto delete branch after MR merge</option>
                            </select>
                        </div>

                        {prTemplate && (
                            <div>
                                <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Detected PR Template</label>
                                <div className="w-full bg-black/40 border border-white/5 rounded-lg p-3 font-mono text-[10px] text-slate-400 max-h-[100px] overflow-y-auto leading-relaxed">
                                    {prTemplate}
                                </div>
                            </div>
                        )}
                    </div>

                    <div className="mt-6 flex justify-end gap-3 border-t border-white/5 pt-4">
                        <button onClick={() => setBootstrapStep('form')} className="px-5 py-2.5 text-sm font-medium text-slate-400 hover:text-white transition-colors">Back</button>
                        <button onClick={handleApproveStrategy} className="px-6 py-2.5 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg shadow-[0_0_15px_rgba(16,185,129,0.3)] transition-all flex items-center gap-2">
                            <Check className="w-4 h-4" /> Approve & Build Workspace
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="flex-1 overflow-y-auto p-8 relative flex flex-col justify-start max-w-4xl mx-auto w-full">
            <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-violet-600/5 rounded-full blur-[120px] pointer-events-none"></div>

            {/* Repo Selection Modal */}
            {isRepoModalOpen && (
                <div className="fixed inset-0 z-50 flex items-center justify-center bg-[#08090c]/80 backdrop-blur-sm">
                    <div className="glass-panel w-[500px] border border-white/10 rounded-xl overflow-hidden shadow-2xl flex flex-col">
                        <div className="p-4 border-b border-white/5 flex justify-between items-center bg-[#0d0f14]">
                            <h3 className="font-outfit font-semibold text-white">Select Repositories</h3>
                            <button onClick={() => setIsRepoModalOpen(false)} className="text-slate-400 hover:text-white p-1 rounded hover:bg-white/5 transition-colors">
                                <X className="w-5 h-5" />
                            </button>
                        </div>
                        <div className="p-4 border-b border-white/5 bg-[#08090c]">
                            <div className="relative">
                                <Search className="w-4 h-4 absolute left-3 top-3 text-slate-500" />
                                <input
                                    type="text"
                                    value={repoSearch}
                                    onChange={(e) => setRepoSearch(e.target.value)}
                                    placeholder="Search repositories..."
                                    className="w-full bg-black/40 border border-white/10 rounded-lg py-2.5 pl-9 pr-4 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                />
                            </div>
                        </div>
                        <div className="overflow-y-auto max-h-[300px] p-2 space-y-1 bg-[#08090c]">
                            {isLoadingRepos ? (
                                <div className="p-4 text-center text-sm text-slate-500">Fetching repositories from connected providers...</div>
                            ) : availableRepos.length === 0 ? (
                                <div className="p-4 text-center text-sm text-slate-500">No repositories found. Make sure providers are connected.</div>
                            ) : availableRepos.filter(r => r.path.toLowerCase().includes(repoSearch.toLowerCase())).map(repo => {
                                const isSelected = selectedRepos.some(r => r.path === repo.path);
                                return (
                                    <div
                                        key={repo.path}
                                        onClick={() => toggleRepo(repo)}
                                        className={`flex items-center gap-3 p-3 rounded-lg cursor-pointer transition-all ${isSelected ? 'bg-cyan-500/10 border border-cyan-500/30' : 'hover:bg-white/5 border border-transparent'
                                            }`}
                                    >
                                        <div className={`w-4 h-4 rounded border flex items-center justify-center ${isSelected ? 'bg-cyan-500 border-cyan-500 text-black' : 'border-slate-600'
                                            }`}>
                                            {isSelected && <Check className="w-3 h-3 stroke-[3]" />}
                                        </div>
                                        <Box className={`w-4 h-4 ${isSelected ? 'text-cyan-400' : 'text-slate-500'}`} />
                                        <span className={isSelected ? 'text-white' : 'text-slate-300'}>{repo.path}</span>
                                    </div>
                                );
                            })}
                        </div>
                        <div className="p-4 border-t border-white/5 flex justify-end gap-3 bg-[#0d0f14]">
                            <button onClick={() => setIsRepoModalOpen(false)} className="px-4 py-2 text-sm font-medium bg-cyan-600 hover:bg-cyan-500 text-white rounded-md transition-colors">Done</button>
                        </div>
                    </div>
                </div>
            )}

            {/* Dirty Warning Modal */}
            {dirtyWarningRepos.length > 0 && (
                <div className="fixed inset-0 z-50 flex items-center justify-center bg-[#08090c]/80 backdrop-blur-sm">
                    <div className="glass-panel w-[500px] border border-ruby-500/20 rounded-xl overflow-hidden shadow-2xl flex flex-col p-6 space-y-4">
                        <div className="flex items-center gap-3 text-ruby-400">
                            <AlertTriangle className="w-8 h-8 shrink-0 animate-pulse" />
                            <h3 className="font-outfit font-bold text-lg text-white">Potential Data Loss Warning</h3>
                        </div>
                        <p className="text-sm text-slate-300 leading-relaxed">
                            {pendingActionAfterConfirm === 'delete' 
                              ? "The workspace has repositories with uncommitted changes or unpushed commits. Deleting the workspace will permanently erase these directories:"
                              : "You are about to remove the following repositories, but they contain uncommitted changes or unpushed commits on the local server. Removing them will permanently erase these folders:"
                            }
                        </p>
                        <div className="bg-black/40 border border-white/5 rounded-lg p-3 max-h-[200px] overflow-y-auto space-y-2">
                            {dirtyWarningRepos.map(repo => (
                                <div key={repo.repo_path.toString()} className="text-xs font-mono p-2 border border-white/5 rounded bg-[#0a0c10]">
                                    <div className="text-white font-medium truncate mb-1">{repo.repo_path}</div>
                                    <div className="flex gap-2">
                                        {repo.has_uncommitted && (
                                            <span className="px-1.5 py-0.5 rounded bg-ruby-500/10 border border-ruby-500/20 text-ruby-400">Uncommitted Changes</span>
                                        )}
                                        {repo.has_unpushed && (
                                            <span className="px-1.5 py-0.5 rounded bg-violet-500/10 border border-violet-500/20 text-violet-400">Unpushed Commits</span>
                                        )}
                                    </div>
                                </div>
                            ))}
                        </div>
                        <p className="text-xs text-slate-400">
                            Are you absolutely sure you want to proceed and permanently delete these files?
                        </p>
                        <div className="flex justify-end gap-3 pt-2">
                            <button 
                                onClick={() => {
                                    setDirtyWarningRepos([]);
                                    setPendingActionAfterConfirm(null);
                                }}
                                className="px-4 py-2 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all"
                            >
                                Cancel
                            </button>
                            <button 
                                onClick={async () => {
                                    if (pendingActionAfterConfirm === 'delete') {
                                        await proceedWithDelete();
                                    } else {
                                        setDirtyWarningRepos([]);
                                        setPendingActionAfterConfirm(null);
                                        await proceedWithReBootstrap();
                                    }
                                }}
                                className="px-4 py-2 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg font-medium transition-all"
                            >
                                Proceed Anyway
                            </button>
                        </div>
                    </div>
                </div>
            )}

            {/* Standard Delete Confirm Modal */}
            {showDeleteConfirm && (
                <div className="fixed inset-0 z-50 flex items-center justify-center bg-[#08090c]/80 backdrop-blur-sm">
                    <div className="glass-panel w-[450px] border border-ruby-500/20 rounded-xl overflow-hidden shadow-2xl flex flex-col p-6 space-y-4">
                        <div className="flex items-center gap-3 text-ruby-400">
                            <Trash2 className="w-7 h-7 shrink-0" />
                            <h3 className="font-outfit font-bold text-lg text-white">Delete Workspace</h3>
                        </div>
                        <p className="text-sm text-slate-300 leading-relaxed">
                            Are you sure you want to delete <span className="text-white font-semibold">{projectName}</span>? This will permanently delete the project record and remove all local workspace clones.
                        </p>
                        <div className="flex justify-end gap-3 pt-2">
                            <button 
                                onClick={() => setShowDeleteConfirm(false)}
                                className="px-4 py-2 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all"
                            >
                                Cancel
                            </button>
                            <button 
                                onClick={proceedWithDelete}
                                className="px-4 py-2 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg font-medium transition-all"
                            >
                                Delete Permanently
                            </button>
                        </div>
                    </div>
                </div>
            )}

            {/* Header Area */}
            <div className="flex justify-between items-center mb-8 border-b border-white/5 pb-4 z-10">
                <div>
                    <h1 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Settings</h1>
                    <p className="text-sm text-slate-400">Configure code isolation rules, repositories and environment configurations for <span className="text-white font-medium">{activeProject.name}</span>.</p>
                </div>
                <button
                    onClick={handleSave}
                    disabled={status === 'saving'}
                    className="bg-cyan-600 hover:bg-cyan-500 disabled:bg-cyan-600/50 text-white font-medium text-sm px-4 py-2 rounded-lg transition-all shadow-[0_0_15px_rgba(6,182,212,0.3)] flex items-center gap-2"
                >
                    {status === 'saving' ? (
                        <RotateCw className="w-4 h-4 animate-spin" />
                    ) : (
                        <Save className="w-4 h-4" />
                    )}
                    Save Changes
                </button>
            </div>

            {activeProject.status === 'error' && (
                <div className="mb-6 bg-ruby-500/10 border border-ruby-500/20 rounded-xl p-4 flex items-start gap-3 shadow-lg z-10">
                    <AlertTriangle className="w-5 h-5 text-ruby-400 shrink-0 mt-0.5 animate-pulse" />
                    <div>
                        <h4 className="font-outfit font-bold text-white text-sm">Workspace Bootstrap Failed</h4>
                        <p className="text-xs text-slate-300 mt-1">
                            The build for this workspace could not complete. Verify target compute availability, credentials, and mapped repository paths, then click <strong>Save Changes</strong> to retry the build.
                        </p>
                    </div>
                </div>
            )}

            {/* Tabs Selector */}
            <div className="flex border-b border-white/5 mb-6 z-10">
                <button
                    onClick={() => setActiveTab('general')}
                    className={`px-4 py-2.5 text-sm font-outfit font-medium border-b-2 transition-all ${activeTab === 'general' ? 'border-cyan-500 text-cyan-400' : 'border-transparent text-slate-400 hover:text-slate-200'}`}
                >
                    General & Repositories
                </button>
                <button
                    onClick={() => setActiveTab('strategy')}
                    className={`px-4 py-2.5 text-sm font-outfit font-medium border-b-2 transition-all ${activeTab === 'strategy' ? 'border-cyan-500 text-cyan-400' : 'border-transparent text-slate-400 hover:text-slate-200'}`}
                >
                    Agent Strategy & Policies
                </button>
                <button
                    onClick={() => setActiveTab('overrides')}
                    className={`px-4 py-2.5 text-sm font-outfit font-medium border-b-2 transition-all ${activeTab === 'overrides' ? 'border-cyan-500 text-cyan-400' : 'border-transparent text-slate-400 hover:text-slate-200'}`}
                >
                    Workflow Overrides
                </button>
                <button
                    onClick={() => setActiveTab('memory')}
                    className={`px-4 py-2.5 text-sm font-outfit font-medium border-b-2 transition-all ${activeTab === 'memory' ? 'border-cyan-500 text-cyan-400' : 'border-transparent text-slate-400 hover:text-slate-200'}`}
                >
                    Project Memory
                </button>
            </div>

            <div className="z-10">
                {activeTab === 'general' ? (
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                        {/* Basic Workspace Form */}
                        <div className="glass-panel p-6 rounded-xl space-y-4">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                <Globe className="w-4 h-4 text-violet-400" /> General Configuration
                            </h3>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Workspace Name</label>
                                <input 
                                    type="text" 
                                    value={projectName} 
                                    onChange={e => setProjectName(e.target.value)}
                                    className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                />
                            </div>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Environment / Target Server</label>
                                <div className="flex gap-2">
                                    <button
                                        onClick={() => setComputeType('local')}
                                        className={`flex-1 flex items-center justify-center gap-2 border rounded-lg py-2 px-3 text-sm transition-all ${computeType === 'local' ? 'bg-violet-500/10 border-violet-500/50 text-violet-300' : 'bg-black/40 border-white/5 text-slate-400'}`}
                                    >
                                        <HardDrive className="w-4 h-4" /> Local Compute
                                    </button>
                                    <button
                                        onClick={() => setComputeType('remote')}
                                        className={`flex-1 flex items-center justify-center gap-2 border rounded-lg py-2 px-3 text-sm transition-all ${computeType === 'remote' ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-300' : 'bg-black/40 border-white/5 text-slate-400'}`}
                                    >
                                        <Server className="w-4 h-4" /> Remote SSH
                                    </button>
                                </div>
                                {computeType === 'remote' && (
                                    <div className="mt-3 flex gap-2">
                                        <select
                                            value={remoteHost}
                                            onChange={e => setRemoteHost(e.target.value)}
                                            className="flex-1 bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white font-mono focus:outline-none focus:border-cyan-500/50"
                                        >
                                            <option value="">
                                                {machines.length === 0 ? 'No machines configured' : 'Select a machine…'}
                                            </option>
                                            {machines.map(m => (
                                                <option key={m.id} value={m.id}>
                                                    {m.name} ({m.username}@{m.host}:{m.port})
                                                </option>
                                            ))}
                                        </select>
                                        <button
                                            type="button"
                                            onClick={handleTestConnection}
                                            disabled={!remoteHost || isTestingConnection}
                                            className="px-4 py-2 text-xs font-semibold rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-white disabled:opacity-40 flex items-center gap-1.5 transition-all shrink-0"
                                        >
                                            {isTestingConnection ? (
                                                <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                                            ) : connectionStatus === 'success' ? (
                                                <Check className="w-3.5 h-3.5 text-emerald-400" />
                                            ) : connectionStatus === 'error' ? (
                                                <X className="w-3.5 h-3.5 text-ruby-400" />
                                            ) : null}
                                            {isTestingConnection ? 'Testing...' : connectionStatus === 'success' ? 'Connected' : connectionStatus === 'error' ? 'Failed' : 'Test'}
                                        </button>
                                        <button
                                            type="button"
                                            onClick={() => setView('settings')}
                                            className="px-3 py-2 text-xs rounded-lg bg-violet-500/10 border border-violet-500/30 hover:bg-violet-500/20 text-violet-300 transition-all shrink-0"
                                            title="Manage machines in Settings"
                                        >
                                            <Settings className="w-3.5 h-3.5" />
                                        </button>
                                    </div>
                                )}
                            </div>
                        </div>

                        {/* Repositories selection */}
                        <div className="glass-panel p-6 rounded-xl space-y-4">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                <Box className="w-4 h-4 text-cyan-400" /> Repositories Mapped ({selectedRepos.length})
                            </h3>

                            <div className="space-y-2 max-h-[190px] overflow-y-auto pr-1">
                                {selectedRepos.length === 0 ? (
                                    <p className="text-xs text-slate-500 italic py-2">No repositories configured.</p>
                                ) : selectedRepos.map(repo => (
                                    <div key={repo.path} className="flex items-center gap-2 p-2 border border-white/5 rounded-lg bg-black/20">
                                        <Box className="w-3.5 h-3.5 text-cyan-400 shrink-0" />
                                        <span className="text-xs text-slate-300 truncate w-4/5">{repo.path}</span>
                                        <button 
                                            onClick={() => toggleRepo(repo)}
                                            className="ml-auto text-slate-500 hover:text-ruby-400 p-0.5 rounded hover:bg-white/5"
                                        >
                                            <X className="w-3.5 h-3.5" />
                                        </button>
                                    </div>
                                ))}
                            </div>

                            <button 
                                onClick={() => {
                                    setIsRepoModalOpen(true);
                                    fetchAllReposFromProviders();
                                }} 
                                className="w-full flex items-center justify-center gap-1.5 p-2 rounded-lg border border-dashed border-white/10 text-slate-400 hover:text-white hover:bg-white/5 transition-all text-xs"
                            >
                                <Plus className="w-3.5 h-3.5" /> Manage Workspace Repositories
                            </button>
                        </div>

                        {/* Workspace Health Panel */}
                        <div className="md:col-span-2">
                            {healthError && (
                                <div className="rounded-xl border border-ruby-500/30 bg-ruby-500/5 p-4 mb-3">
                                    <div className="flex items-center gap-2 mb-2">
                                        <AlertTriangle className="w-4 h-4 text-ruby-400" />
                                        <span className="font-outfit text-sm font-semibold text-ruby-300 uppercase tracking-wider">Workspace Health Check Failed</span>
                                    </div>
                                    <pre className="font-mono text-xs text-ruby-200/80 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">
{healthError}
                                    </pre>
                                    <div className="mt-3 flex gap-2">
                                        <button
                                            onClick={fetchWorkspaceHealth}
                                            disabled={isLoadingHealth}
                                            className="px-3 py-1.5 text-xs rounded-md border border-ruby-500/30 text-ruby-200 hover:bg-ruby-500/10 transition-all flex items-center gap-1.5"
                                        >
                                            <RefreshCw className={`w-3 h-3 ${isLoadingHealth ? 'animate-spin' : ''}`} /> Retry
                                        </button>
                                        <button
                                            onClick={proceedWithReBootstrap}
                                            className="px-3 py-1.5 text-xs rounded-md bg-cyan-600 hover:bg-cyan-500 text-white transition-all font-medium"
                                        >
                                            Re-run Bootstrap
                                        </button>
                                    </div>
                                </div>
                            )}
                            {!showHealthPanel ? (
                                <button
                                    onClick={fetchWorkspaceHealth}
                                    disabled={isLoadingHealth}
                                    className="w-full flex items-center justify-center gap-2 p-3 rounded-xl border border-dashed border-white/10 text-slate-400 hover:text-cyan-400 hover:border-cyan-500/30 hover:bg-cyan-500/5 transition-all text-sm"
                                >
                                    {isLoadingHealth ? <RotateCw className="w-4 h-4 animate-spin" /> : <Activity className="w-4 h-4" />}
                                    {isLoadingHealth ? 'Checking workspace health...' : 'Check Workspace Health'}
                                </button>
                            ) : (
                                <div className="glass-panel rounded-xl border border-white/5 overflow-hidden">
                                    {/* Health panel header — note: cannot be a
                                        <button> because we nest a refresh
                                        <button> inside it. Use a div with
                                        role=button + tabIndex + onKeyDown so
                                        the row stays clickable for the
                                        expand/collapse toggle. */}
                                    <div
                                        role="button"
                                        tabIndex={0}
                                        onClick={() => setHealthExpanded(prev => !prev)}
                                        onKeyDown={(e) => {
                                            if (e.key === 'Enter' || e.key === ' ') {
                                                e.preventDefault();
                                                setHealthExpanded(prev => !prev);
                                            }
                                        }}
                                        className="w-full flex items-center justify-between px-5 py-3.5 bg-white/[0.02] hover:bg-white/[0.04] transition-colors cursor-pointer"
                                    >
                                        <div className="flex items-center gap-2.5">
                                            <Activity className="w-4 h-4 text-cyan-400" />
                                            <span className="font-outfit text-sm font-semibold text-slate-200 uppercase tracking-wider">Workspace Health</span>
                                            {healthData && healthData.length > 0 && (() => {
                                                const hasError = healthData.some(r => !r.is_cloned);
                                                const hasDirty = healthData.some(r => r.has_uncommitted || r.has_unpushed);
                                                if (hasError) return <span className="px-2 py-0.5 text-[10px] rounded-full bg-ruby-500/15 border border-ruby-500/25 text-ruby-400 font-mono">DEGRADED</span>;
                                                if (hasDirty) return <span className="px-2 py-0.5 text-[10px] rounded-full bg-amber-500/15 border border-amber-500/25 text-amber-400 font-mono">DIRTY</span>;
                                                return <span className="px-2 py-0.5 text-[10px] rounded-full bg-emerald-500/15 border border-emerald-500/25 text-emerald-400 font-mono">HEALTHY</span>;
                                            })()}
                                        </div>
                                        <div className="flex items-center gap-2">
                                            <button
                                                onClick={(e) => { e.stopPropagation(); fetchWorkspaceHealth(); }}
                                                disabled={isLoadingHealth}
                                                className="p-1.5 rounded-md text-slate-400 hover:text-cyan-400 hover:bg-white/5 transition-all"
                                                title="Refresh health"
                                            >
                                                <RefreshCw className={`w-3.5 h-3.5 ${isLoadingHealth ? 'animate-spin' : ''}`} />
                                            </button>
                                            {healthExpanded ? <ChevronUp className="w-4 h-4 text-slate-500" /> : <ChevronDown className="w-4 h-4 text-slate-500" />}
                                        </div>
                                    </div>

                                    {healthExpanded && (
                                        <div className="p-4 space-y-3">
                                            {isLoadingHealth && !healthData ? (
                                                <div className="flex items-center gap-2 text-sm text-slate-400 py-2">
                                                    <RotateCw className="w-4 h-4 animate-spin text-cyan-400" />
                                                    Scanning repositories...
                                                </div>
                                            ) : healthData && healthData.length > 0 ? (
                                                healthData.map(repo => {
                                                    const repoName = repo.repo_path.split('/').pop() ?? repo.repo_path;
                                                    // Active worktrees = all except the first (main) worktree
                                                    const activeWorktrees = repo.worktrees.slice(1);
                                                    const isDirty = repo.has_uncommitted || repo.has_unpushed;
                                                    return (
                                                        <div key={repo.repo_path} className={`rounded-lg border p-3.5 transition-all ${repo.is_cloned ? (isDirty ? 'border-amber-500/20 bg-amber-500/5' : 'border-white/5 bg-black/20') : 'border-ruby-500/20 bg-ruby-500/5'}`}>
                                                            <div className="flex items-center gap-3 flex-wrap">
                                                                {/* Status dot */}
                                                                <span className={`w-2 h-2 rounded-full shrink-0 ${repo.is_cloned ? (isDirty ? 'bg-amber-400' : 'bg-emerald-400') : 'bg-ruby-400 animate-pulse'}`} />

                                                                {/* Repo name */}
                                                                <div className="flex flex-col min-w-0 flex-1">
                                                                    <span className="text-sm text-white font-medium truncate">{repoName}</span>
                                                                    <span className="text-[11px] text-slate-500 truncate">{repo.repo_path}</span>
                                                                </div>

                                                                {/* Badges */}
                                                                <div className="flex items-center gap-1.5 flex-wrap justify-end">
                                                                    {repo.is_cloned ? (
                                                                        <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">CLONED</span>
                                                                    ) : (
                                                                        <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-ruby-500/10 border border-ruby-500/20 text-ruby-400">MISSING</span>
                                                                    )}
                                                                    {repo.head_branch && (
                                                                        <span className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-mono rounded-md bg-cyan-500/10 border border-cyan-500/20 text-cyan-400">
                                                                            <GitBranch className="w-2.5 h-2.5" />{repo.head_branch}
                                                                        </span>
                                                                    )}
                                                                    {activeWorktrees.length > 0 && (
                                                                        <span className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-mono rounded-md bg-violet-500/10 border border-violet-500/20 text-violet-400">
                                                                            <GitBranch className="w-2.5 h-2.5" />{activeWorktrees.length} worktree{activeWorktrees.length !== 1 ? 's' : ''}
                                                                        </span>
                                                                    )}
                                                                    {repo.has_uncommitted && (
                                                                        <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-400">Uncommitted</span>
                                                                    )}
                                                                    {repo.has_unpushed && (
                                                                        <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-orange-500/10 border border-orange-500/20 text-orange-400">Unpushed</span>
                                                                    )}
                                                                </div>
                                                            </div>

                                                            {/* Active worktrees list */}
                                                            {activeWorktrees.length > 0 && (
                                                                <div className="mt-2.5 pl-5 space-y-1">
                                                                    {activeWorktrees.map(wt => (
                                                                        <div key={wt.path} className="flex items-center gap-2 text-[10px] text-slate-400">
                                                                            <span className="w-1 h-1 rounded-full bg-violet-400 shrink-0" />
                                                                            <span className="font-mono truncate">{wt.branch ?? wt.path.split('/').pop()}</span>
                                                                            {wt.is_locked && <span className="text-amber-400">(locked)</span>}
                                                                        </div>
                                                                    ))}
                                                                </div>
                                                            )}

                                                            {/* Re-run Bootstrap shortcut for missing repos */}
                                                            {!repo.is_cloned && (
                                                                <div className="mt-3 flex items-center gap-2 p-2.5 rounded-lg bg-ruby-500/5 border border-ruby-500/15">
                                                                    <CircleAlert className="w-4 h-4 text-ruby-400 shrink-0" />
                                                                    <p className="text-[11px] text-slate-400 flex-1">This repository clone is missing. Re-run bootstrap to restore it.</p>
                                                                    <button
                                                                        onClick={proceedWithReBootstrap}
                                                                        className="px-3 py-1.5 text-[11px] font-semibold bg-ruby-600/80 hover:bg-ruby-500 text-white rounded-md transition-all flex items-center gap-1.5 shrink-0"
                                                                    >
                                                                        <RotateCw className="w-3 h-3" /> Re-bootstrap
                                                                    </button>
                                                                </div>
                                                            )}
                                                        </div>
                                                    );
                                                })
                                            ) : (
                                                <p className="text-sm text-slate-500 italic py-1">No repository data available.</p>
                                            )}
                                        </div>
                                    )}
                                </div>
                            )}
                        </div>

                        {/* Danger zone card */}
                        <div className="glass-panel p-6 rounded-xl border border-ruby-500/10 md:col-span-2 flex flex-col md:flex-row items-center justify-between gap-4">
                            <div className="flex gap-3">
                                <Trash2 className="w-10 h-10 text-ruby-400 shrink-0" />
                                <div>
                                    <h4 className="font-outfit font-bold text-white text-base">Danger Zone: Destroy Workspace</h4>
                                    <p className="text-xs text-slate-400 mt-1 max-w-xl">
                                        Deleting a workspace will remove its configuration records and permanently delete all local repository clones. This action is irreversible.
                                    </p>
                                </div>
                            </div>
                            <button
                                onClick={handleDeleteClick}
                                className="bg-ruby-600 hover:bg-ruby-500 text-white font-semibold text-xs px-4 py-2.5 rounded-lg transition-all shrink-0 shadow-[0_0_15px_rgba(239,68,68,0.2)]"
                            >
                                Delete Workspace
                            </button>
                        </div>
                    </div>
                ) : activeTab === 'strategy' ? (
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                        <div className="glass-panel p-6 rounded-xl space-y-4">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                <GitBranch className="w-4 h-4 text-violet-400" /> Git Isolation & Strategy
                            </h3>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Branch</label>
                                <input 
                                    type="text" 
                                    value={defaultBranch} 
                                    onChange={e => setDefaultBranch(e.target.value)}
                                    className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                />
                            </div>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Branch Prefix</label>
                                <input 
                                    type="text" 
                                    value={branchPrefix} 
                                    onChange={e => setBranchPrefix(e.target.value)}
                                    className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                />
                            </div>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Test Command</label>
                                <input 
                                    type="text" 
                                    value={testCommand} 
                                    onChange={e => setTestCommand(e.target.value)}
                                    placeholder="e.g. npm test or cargo test"
                                    className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                                />
                            </div>
                        </div>

                        {/* Test Harnesses Card */}
                        <div className="glass-panel p-6 rounded-xl space-y-4">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                <Zap className="w-4 h-4 text-cyan-400" /> Named Test Harnesses
                            </h3>
                            <p className="text-xs text-slate-400 leading-relaxed">
                                Define named test harness commands to verify agent-generated code (e.g., key: <code>lint</code>, command: <code>npm run lint</code>).
                            </p>

                            <div className="space-y-3">
                                {Object.entries(harnesses).map(([name, cmd]) => (
                                    <div key={name} className="flex gap-2 items-center">
                                        <div className="flex-1 font-mono text-xs bg-black/40 border border-white/10 rounded-lg p-2 text-white truncate">
                                            <span className="text-cyan-400">{name}</span>: <span className="text-slate-300">{cmd}</span>
                                        </div>
                                        <button
                                            type="button"
                                            onClick={() => {
                                                const copy = { ...harnesses };
                                                delete copy[name];
                                                setHarnesses(copy);
                                            }}
                                            className="p-2 text-slate-500 hover:text-ruby-400 bg-white/5 rounded-lg border border-white/5 hover:bg-white/10 shrink-0"
                                            title="Delete harness"
                                        >
                                            <Trash2 className="w-3.5 h-3.5" />
                                        </button>
                                    </div>
                                ))}

                                <div className="border-t border-white/5 pt-3 flex gap-2">
                                    <input
                                        type="text"
                                        placeholder="Name"
                                        id="new-harness-name"
                                        className="w-1/3 bg-black/40 border border-white/10 rounded-lg py-1.5 px-3 text-xs text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50 font-mono"
                                    />
                                    <input
                                        type="text"
                                        placeholder="Command"
                                        id="new-harness-cmd"
                                        className="flex-1 bg-black/40 border border-white/10 rounded-lg py-1.5 px-3 text-xs text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50 font-mono"
                                    />
                                    <button
                                        type="button"
                                        onClick={() => {
                                            const nameEl = document.getElementById('new-harness-name') as HTMLInputElement;
                                            const cmdEl = document.getElementById('new-harness-cmd') as HTMLInputElement;
                                            if (nameEl && cmdEl) {
                                                const name = nameEl.value.trim();
                                                const cmd = cmdEl.value.trim();
                                                if (name && cmd) {
                                                    setHarnesses(prev => ({ ...prev, [name]: cmd }));
                                                    nameEl.value = '';
                                                    cmdEl.value = '';
                                                }
                                            }
                                        }}
                                        className="px-3 py-1.5 text-xs bg-cyan-600 hover:bg-cyan-500 text-white rounded-lg transition-colors flex items-center gap-1 font-semibold shrink-0"
                                    >
                                        <Plus className="w-3 h-3" /> Add
                                    </button>
                                </div>
                            </div>
                        </div>

                        <div className="glass-panel p-6 rounded-xl space-y-4">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                <Settings className="w-4 h-4 text-cyan-400" /> Automation Policies
                            </h3>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Conflict Resolution Policy</label>
                                <select 
                                    value={conflictPolicy} 
                                    onChange={e => setConflictPolicy(e.target.value)}
                                    className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                >
                                    <option value="always_gate">Always Gate (Requires approval)</option>
                                    <option value="auto_agent">Auto Agent First (Cascade to manual)</option>
                                    <option value="auto_human">Immediate Manual Merge</option>
                                </select>
                            </div>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Completed Feature Lifecycle</label>
                                <select 
                                    value={featureLifecycle} 
                                    onChange={e => setFeatureLifecycle(e.target.value)}
                                    className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                >
                                    <option value="archive">Archive by default</option>
                                    <option value="keep">Keep active</option>
                                    <option value="auto_delete">Auto delete branch after MR merge</option>
                                </select>
                            </div>
                        </div>

                        {/* Default AI Executor Settings */}
                        <div className="glass-panel p-6 rounded-xl space-y-4">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                <Zap className="w-4 h-4 text-violet-400" /> Default AI Executor Settings
                            </h3>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Coding Agent</label>
                                <select 
                                    value={defaultAgentKind} 
                                    onChange={e => {
                                        setDefaultAgentKind(e.target.value);
                                        setDefaultModel('');
                                    }}
                                    className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 capitalize"
                                >
                                    <option value="">No default (Prompt on feature creation)</option>
                                    {agentConfigs.filter(a => a.enabled && a.available && a.kind !== 'antigravity').map(a => (
                                        <option key={a.kind} value={a.kind}>
                                            {a.kind.replace(/-/g, ' ')}
                                        </option>
                                    ))}
                                </select>
                            </div>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Model</label>
                                <div>
                                    {isLoadingModelsForDefault ? (
                                        <div className="w-full bg-[#08090c]/40 border border-white/10 rounded-lg py-2.5 px-3 text-sm text-slate-400 flex items-center gap-2">
                                            <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                                            <span>Probing available models...</span>
                                        </div>
                                    ) : (
                                        <div className="flex gap-2">
                                            <select
                                                value={defaultModel}
                                                onChange={e => setDefaultModel(e.target.value)}
                                                className="flex-1 min-w-0 bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                                disabled={!defaultAgentKind}
                                            >
                                                <option value="">No default model</option>
                                                {availableModelsForDefault.map(m => (
                                                    <option key={m.value} value={m.value}>{m.name}</option>
                                                ))}
                                                {defaultModel && !availableModelsForDefault.some(m => m.value === defaultModel) && (
                                                    <option value={defaultModel}>{defaultModel} (custom)</option>
                                                )}
                                            </select>
                                            <input
                                                type="text"
                                                value={defaultModel}
                                                onChange={e => setDefaultModel(e.target.value)}
                                                placeholder="Custom override"
                                                className="w-1/3 shrink-0 min-w-[140px] bg-black/40 border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600"
                                                disabled={!defaultAgentKind}
                                            />
                                        </div>
                                    )}
                                </div>
                                <div className="mt-4">
                                    <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                                        Default Loop Iterations
                                    </label>
                                    <input
                                        type="number"
                                        min={1}
                                        max={10}
                                        value={defaultLoopIterations}
                                        onChange={e => setDefaultLoopIterations(e.target.value)}
                                        placeholder="3 (engine default)"
                                        className="w-40 bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600"
                                    />
                                    <p className="text-[11px] text-slate-500 mt-1.5 leading-relaxed">
                                        How many times a validation step may loop back to implementation before giving up. Leave blank to use the engine default (3). Overridable per run.
                                    </p>
                                </div>
                            </div>
                        </div>

                        {/* Artifact Handling (migration V12) */}
                        <div className="glass-panel p-6 rounded-xl space-y-4">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                <FileText className="w-4 h-4 text-cyan-400" /> Artifact Handling
                            </h3>
                            <p className="text-xs text-slate-400 leading-relaxed">
                                Each workflow step produces a report (<code className="text-slate-300">research-report.md</code>, <code className="text-slate-300">critic-review.md</code>, …). By default these land in a subfolder and stay out of the PR — view them in demeteo's artifact panel. Toggle the commit switch to ship them with the feature branch instead.
                            </p>

                            <div>
                                <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                                    Artifact Subfolder
                                </label>
                                <input
                                    type="text"
                                    value={artifactSubdir}
                                    onChange={e => setArtifactSubdir(e.target.value)}
                                    placeholder="artifacts/"
                                    className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600"
                                />
                                <p className="text-[10px] font-mono text-slate-500 mt-1.5 leading-relaxed">
                                    Repo-relative path. The orchestrator injects this as <code>{'{{artifact_dir}}'}</code> into every step's prompt and excludes it from <code>git add</code> when the commit switch is off.
                                </p>
                            </div>

                            <label className="flex items-start gap-3 p-3 rounded-lg border border-white/5 bg-black/20 cursor-pointer hover:border-cyan-500/30 transition-colors">
                                <input
                                    type="checkbox"
                                    checked={commitArtifacts}
                                    onChange={e => setCommitArtifacts(e.target.checked)}
                                    className="mt-0.5 w-4 h-4 rounded border-white/20 bg-black/40 text-cyan-500 focus:ring-cyan-500/40 focus:ring-offset-0"
                                />
                                <div className="flex-1">
                                    <div className="text-xs font-semibold text-slate-200">
                                        Commit artifacts to the feature branch
                                    </div>
                                    <div className="text-[11px] text-slate-400 mt-0.5 leading-relaxed">
                                        When off (default), the orchestrator runs <code>git add -A -- ':!&lt;artifact_subfolder&gt;'</code> so the reports stay as untracked files in the worktree. The UI viewer still shows them.
                                    </div>
                                </div>
                            </label>
                        </div>

                        {prTemplate && (
                            <div className="glass-panel p-6 rounded-xl md:col-span-2 space-y-2">
                                <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider">Detected PR Template</h3>
                                <div className="w-full bg-black/40 border border-white/5 rounded-lg p-4 font-mono text-xs text-slate-400 max-h-[160px] overflow-y-auto leading-relaxed">
                                    {prTemplate}
                                </div>
                            </div>
                        )}

                        {/* Coding Agent Configurations */}
                        <div className="glass-panel p-6 rounded-xl md:col-span-2 space-y-4">
                            <div className="flex items-center justify-between border-b border-white/5 pb-2">
                                <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
                                    <Activity className="w-4 h-4 text-cyan-400 animate-pulse" /> Coding Agent Configuration
                                </h3>
                                <button
                                    type="button"
                                    onClick={() => fetchAgentConfigs(true)}
                                    disabled={isRefreshingAgents}
                                    className="p-1 rounded text-slate-500 hover:text-cyan-400 hover:bg-white/5 transition-all disabled:opacity-50"
                                    title="Re-check agent availability"
                                >
                                    <RotateCw className={`w-3.5 h-3.5 ${isRefreshingAgents ? 'animate-spin text-cyan-400' : ''}`} />
                                </button>
                            </div>
                            <p className="text-xs text-slate-400">
                                Enable or disable specific AI coding agents for this workspace. Demeteo validates if these agents' CLI binaries are available on the selected compute server.
                            </p>

                            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-2">
                                {agentConfigs.filter(a => a.kind !== 'antigravity').length === 0 ? (
                                    <div className="md:col-span-2 text-xs text-slate-500 italic p-2">No agents found on target machine.</div>
                                ) : agentConfigs.filter(a => a.kind !== 'antigravity').map((agent) => (
                                    <div 
                                        key={agent.kind}
                                        className={`flex items-start justify-between p-4 rounded-lg border transition-all ${
                                            agent.enabled 
                                                ? 'bg-violet-500/5 border-violet-500/25 shadow-[0_0_15px_rgba(139,92,246,0.05)]' 
                                                : 'bg-black/20 border-white/5 opacity-60'
                                        }`}
                                    >
                                        <div className="flex gap-3 w-full">
                                            {/* Checkbox */}
                                            <div className="pt-0.5">
                                                <button
                                                    type="button"
                                                    onClick={() => {
                                                        setAgentConfigs(prev => prev.map(a => 
                                                            a.kind === agent.kind ? { ...a, enabled: !a.enabled } : a
                                                        ));
                                                    }}
                                                    className={`w-4 h-4 rounded border flex items-center justify-center transition-all ${
                                                        agent.enabled 
                                                            ? 'bg-violet-500 border-violet-500 text-white' 
                                                            : 'border-slate-600 hover:border-slate-500'
                                                    }`}
                                                >
                                                    {agent.enabled && <Check className="w-3 h-3 stroke-[3]" />}
                                                </button>
                                            </div>

                                            <div className="flex-1 min-w-0">
                                                <div className="flex items-center gap-2 flex-wrap">
                                                    <span className="text-sm font-semibold text-white font-outfit capitalize">
                                                        {agent.kind.replace(/-/g, ' ')}
                                                    </span>
                                                    {/* Status badge */}
                                                    {agent.available ? (
                                                        <span className="flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 font-mono">
                                                            <span className="w-1.5 h-1.5 rounded-full bg-emerald-400"></span>
                                                            Available
                                                        </span>
                                                    ) : (
                                                        <span className="flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded bg-ruby-500/10 border border-ruby-500/20 text-ruby-400 font-mono">
                                                            <span className="w-1.5 h-1.5 rounded-full bg-ruby-400"></span>
                                                            Missing
                                                        </span>
                                                    )}
                                                </div>
                                                <p className="text-[11px] text-slate-400 mt-1 leading-relaxed">
                                                    {agent.kind === 'opencode' && 'Local open-source developer agent.'}
                                                    {agent.kind === 'hermes' && 'Autonomic codebase planner and execution agent.'}
                                                    {agent.kind === 'claude-code' && 'Claude Code agent for complex tasks.'}
                                                    {agent.kind === 'antigravity' && 'Antigravity coding assistant.'}
                                                    {!['opencode', 'hermes', 'claude-code', 'antigravity'].includes(agent.kind) && 'Additional configured coding agent.'}
                                                </p>
                                                {!agent.available && agent.install_command && (
                                                    <div className="mt-2.5 p-2 bg-black/40 border border-white/5 rounded font-mono text-[9px] text-slate-300 flex items-center justify-between gap-2 select-all overflow-x-auto">
                                                        <span>{agent.install_command}</span>
                                                    </div>
                                                )}
                                            </div>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        </div>
                    </div>
                ) : activeTab === 'overrides' ? (
                    <div className="space-y-4 animate-fadeIn">
                        {/* Intro / explainer */}
                        <div className="glass-panel p-6 rounded-xl space-y-2">
                            <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
                                <WorkflowIcon className="w-4 h-4 text-violet-400" /> Workflow &amp; Step Harness &amp; Model
                            </h3>
                            <p className="text-xs text-slate-400 leading-relaxed">
                                Workflows are shared across projects. Pin a coding agent (<span className="text-slate-300">harness</span>) and model for a whole workflow — or a single step — <span className="text-white font-medium">when it runs in {activeProject.name}</span>. Models are probed live from your {computeType === 'remote' ? 'remote machine' : 'local machine'}, so you only pick what's actually available.
                            </p>
                            <p className="text-[11px] text-slate-500 leading-relaxed">
                                Precedence, most specific first: a choice made at launch → a step override here → the workflow author's step setting → a workflow override here → the project default. Expand a workflow to override individual steps. Changes save instantly.
                            </p>
                        </div>

                        {overridesError && (
                            <div className="bg-ruby-500/10 border border-ruby-500/30 p-3 rounded-lg flex items-start gap-3">
                                <ShieldAlert className="w-5 h-5 text-ruby-400 shrink-0" />
                                <span className="text-sm text-ruby-200">{overridesError}</span>
                            </div>
                        )}

                        {computeType === 'remote' && !remoteHost && (
                            <div className="bg-amber-500/10 border border-amber-500/20 p-3 rounded-lg flex items-start gap-3">
                                <AlertTriangle className="w-5 h-5 text-amber-400 shrink-0" />
                                <span className="text-sm text-amber-200">Select a remote machine in <span className="font-medium">General &amp; Repositories</span> to probe available models.</span>
                            </div>
                        )}

                        {isLoadingOverrides ? (
                            <div className="flex items-center justify-center py-16">
                                <RotateCw className="w-6 h-6 text-cyan-400 animate-spin" />
                            </div>
                        ) : workflows.length === 0 ? (
                            <div className="text-center py-16 border border-dashed border-white/10 rounded-xl bg-black/20">
                                <WorkflowIcon className="w-8 h-8 text-slate-600 mx-auto mb-3" />
                                <p className="text-sm font-medium text-slate-400">No workflows found</p>
                                <p className="text-xs text-slate-500 mt-1">Create a workflow first, then return here to override its harness and model.</p>
                            </div>
                        ) : (
                            <div className="space-y-3">
                                {workflows.map(wf => {
                                    const count = workflowOverrideCount(wf);
                                    const isActive = count > 0;
                                    const expanded = Boolean(expandedWf[wf.id]);
                                    const agentSteps = wf.steps.filter(s => s.kind !== 'gate');
                                    const wfLevel = overrides[ovKey(wf.id, WF_LEVEL)];
                                    return (
                                        <div
                                            key={wf.id}
                                            className={`glass-panel rounded-xl border transition-all ${isActive ? 'border-violet-500/30 bg-violet-500/[0.03]' : 'border-white/5'}`}
                                        >
                                            {/* Header — click to expand */}
                                            <div
                                                role="button"
                                                tabIndex={0}
                                                onClick={() => toggleWorkflowExpanded(wf)}
                                                onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleWorkflowExpanded(wf); } }}
                                                className="flex items-center gap-3 p-5 cursor-pointer select-none"
                                            >
                                                <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 border ${isActive ? 'bg-violet-500/10 border-violet-500/30 text-violet-300' : 'bg-white/5 border-white/10 text-slate-400'}`}>
                                                    <WorkflowIcon className="w-4 h-4" />
                                                </div>
                                                <div className="min-w-0 flex-1">
                                                    <div className="flex items-center gap-2 flex-wrap">
                                                        <span className="text-sm font-semibold text-white truncate">{wf.name}</span>
                                                        {wfLevel?.agent_kind || wfLevel?.model ? (
                                                            <span className="px-2 py-0.5 text-[9px] font-mono rounded-full bg-violet-500/10 border border-violet-500/20 text-violet-300 uppercase tracking-wider shrink-0">All steps</span>
                                                        ) : null}
                                                        {(() => {
                                                            const stepCount = count - (wfLevel?.agent_kind || wfLevel?.model ? 1 : 0);
                                                            return stepCount > 0 ? (
                                                                <span className="px-2 py-0.5 text-[9px] font-mono rounded-full bg-cyan-500/10 border border-cyan-500/20 text-cyan-300 uppercase tracking-wider shrink-0">{stepCount} step{stepCount !== 1 ? 's' : ''}</span>
                                                            ) : null;
                                                        })()}
                                                    </div>
                                                    {wf.description && (
                                                        <p className="text-[11px] text-slate-500 mt-0.5 line-clamp-1">{wf.description}</p>
                                                    )}
                                                </div>
                                                <span className="text-[10px] text-slate-500 font-mono shrink-0">{agentSteps.length} step{agentSteps.length !== 1 ? 's' : ''}</span>
                                                {expanded ? <ChevronUp className="w-4 h-4 text-slate-500 shrink-0" /> : <ChevronDown className="w-4 h-4 text-slate-500 shrink-0" />}
                                            </div>

                                            {expanded && (
                                                <div className="px-5 pb-5 space-y-5 border-t border-white/5 pt-4">
                                                    {/* Workflow-level (all steps) */}
                                                    <div>
                                                        <div className="flex items-center gap-2 mb-2">
                                                            <span className="text-[10px] font-bold text-violet-300/80 uppercase tracking-wider">Applies to all steps</span>
                                                            <div className="h-px flex-1 bg-white/5" />
                                                        </div>
                                                        {renderOverrideRow(wf, null)}
                                                    </div>

                                                    {/* Per-step */}
                                                    {agentSteps.length > 0 && (
                                                        <div>
                                                            <div className="flex items-center gap-2 mb-3">
                                                                <span className="text-[10px] font-bold text-slate-400 uppercase tracking-wider">Per-step overrides</span>
                                                                <div className="h-px flex-1 bg-white/5" />
                                                            </div>
                                                            <div className="space-y-4">
                                                                {agentSteps.map((step, idx) => (
                                                                    <div key={step.id} className="rounded-lg border border-white/5 bg-black/20 p-3.5">
                                                                        <div className="flex items-center gap-2 mb-3">
                                                                            <span className="text-[10px] font-bold px-1.5 py-0.5 rounded bg-white/5 text-slate-400 shrink-0">{idx + 1}</span>
                                                                            <span className="text-xs font-semibold text-slate-200 truncate">{step.title}</span>
                                                                            <span className={`px-1.5 py-0.5 text-[9px] font-mono rounded uppercase tracking-wider shrink-0 ${step.kind === 'parallel' ? 'bg-violet-500/10 text-violet-300' : 'bg-cyan-500/10 text-cyan-300'}`}>{step.kind}</span>
                                                                        </div>
                                                                        {renderOverrideRow(wf, step)}
                                                                    </div>
                                                                ))}
                                                            </div>
                                                        </div>
                                                    )}
                                                </div>
                                            )}
                                        </div>
                                    );
                                })}
                            </div>
                        )}
                    </div>
                ) : (
                    <div className="grid grid-cols-1 md:grid-cols-3 gap-6 animate-fadeIn">
                        {/* Memory Entries List */}
                        <div className="md:col-span-2 space-y-4">
                            <div className="glass-panel p-6 rounded-xl space-y-4">
                                <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                    <Brain className="w-4 h-4 text-violet-400" /> Project Context Memory
                                </h3>
                                <p className="text-xs text-slate-400 leading-relaxed">
                                    This memory stores context, instructions, and lessons learned from past runs. These entries are automatically injected into the agent's prompts for this project.
                                </p>

                                {isMemoriesLoading ? (
                                    <div className="flex items-center justify-center py-8">
                                        <RotateCw className="w-6 h-6 text-cyan-400 animate-spin" />
                                    </div>
                                ) : memories.length === 0 ? (
                                    <div className="text-center py-12 border border-dashed border-white/10 rounded-xl bg-black/20">
                                        <Brain className="w-8 h-8 text-slate-600 mx-auto mb-3" />
                                        <p className="text-sm font-medium text-slate-400">No memory entries found</p>
                                        <p className="text-xs text-slate-500 mt-1">Manual additions or feedback captured from gate re-runs will appear here.</p>
                                    </div>
                                ) : (
                                    <div className="space-y-4 max-h-[600px] overflow-y-auto pr-1">
                                        {memories.map((entry) => (
                                            <div key={entry.id} className="p-4 rounded-xl border border-white/5 bg-[#0a0c10]/60 backdrop-blur-md flex flex-col relative group hover:border-white/10 transition-all">
                                                <div className="flex justify-between items-center mb-2">
                                                    <span className="font-mono text-xs font-bold text-white max-w-[70%] truncate">
                                                        {entry.key}
                                                    </span>
                                                    <div className="flex items-center gap-2">
                                                        <span className={`px-2 py-0.5 text-[9px] font-mono rounded-full ${
                                                            entry.source === 'human' 
                                                                ? 'bg-cyan-500/10 border border-cyan-500/20 text-cyan-400' 
                                                                : 'bg-violet-500/10 border border-violet-500/20 text-violet-400'
                                                        }`}>
                                                            {entry.source.toUpperCase()}
                                                        </span>
                                                        <button
                                                            type="button"
                                                            onClick={() => handleEditMemoryClick(entry)}
                                                            className="p-1 rounded text-slate-400 hover:text-cyan-400 hover:bg-white/5 transition-all opacity-0 group-hover:opacity-100 focus:opacity-100"
                                                            title="Edit entry"
                                                        >
                                                            <Edit className="w-3.5 h-3.5" />
                                                        </button>
                                                        <button
                                                            type="button"
                                                            onClick={() => handleDeleteMemory(entry.id)}
                                                            className="p-1 rounded text-slate-400 hover:text-ruby-400 hover:bg-white/5 transition-all opacity-0 group-hover:opacity-100 focus:opacity-100"
                                                            title="Delete entry"
                                                        >
                                                            <Trash2 className="w-3.5 h-3.5" />
                                                        </button>
                                                    </div>
                                                </div>
                                                <div className="text-xs text-slate-300 font-mono bg-black/40 border border-white/5 rounded-lg p-2.5 whitespace-pre-wrap leading-relaxed">
                                                    {entry.value}
                                                </div>
                                                <div className="mt-2 text-[10px] text-slate-500 flex justify-between">
                                                    <span>Confidence: {(entry.confidence * 100).toFixed(0)}%</span>
                                                    <span>Updated: {new Date(entry.updated_at).toLocaleString()}</span>
                                                </div>
                                            </div>
                                        ))}
                                    </div>
                                )}
                            </div>
                        </div>

                        {/* Add/Edit Memory Form */}
                        <div className="space-y-4">
                            <form onSubmit={handleSaveMemory} className="glass-panel p-5 rounded-xl border border-white/10 space-y-4">
                                <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
                                    {editingMemory ? <Edit className="w-4 h-4 text-cyan-400" /> : <Plus className="w-4 h-4 text-cyan-400" />}
                                    {editingMemory ? 'Edit Memory' : 'Add Memory'}
                                </h3>

                                <div>
                                    <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Memory Key</label>
                                    <input 
                                        type="text" 
                                        value={newMemKey} 
                                        onChange={e => setNewMemKey(e.target.value)}
                                        placeholder="e.g. build_warning_fix"
                                        className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-xs text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600"
                                        disabled={!!editingMemory}
                                    />
                                    {editingMemory && <p className="text-[10px] text-slate-500 mt-1">Keys cannot be changed after creation.</p>}
                                </div>

                                <div>
                                    <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Memory Value / Context</label>
                                    <textarea 
                                        value={newMemVal} 
                                        onChange={e => setNewMemVal(e.target.value)}
                                        placeholder="Enter context, instructions or fixes..."
                                        rows={8}
                                        className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-xs text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600 resize-none"
                                    />
                                </div>

                                {memError && (
                                    <p className="text-xs text-ruby-400 bg-ruby-500/10 border border-ruby-500/20 rounded p-2">{memError}</p>
                                )}

                                <div className="flex gap-2 justify-end pt-2">
                                    {editingMemory && (
                                        <button 
                                            type="button"
                                            onClick={handleCancelEdit} 
                                            className="px-4 py-2 text-xs bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all"
                                        >
                                            Cancel
                                        </button>
                                    )}
                                    <button 
                                        type="submit"
                                        className="px-4 py-2 text-xs bg-cyan-600 hover:bg-cyan-500 text-white font-semibold rounded-lg transition-all flex items-center gap-1.5"
                                    >
                                        <Save className="w-3.5 h-3.5" />
                                        {editingMemory ? 'Save Changes' : 'Add Entry'}
                                    </button>
                                </div>
                            </form>
                        </div>
                    </div>
                )}

                {status === 'error' && (
                    <div className="bg-ruby-500/10 border border-ruby-500/30 p-3 rounded-lg flex items-start gap-3 mt-6">
                        <ShieldAlert className="w-5 h-5 text-ruby-400 shrink-0" />
                        <span className="text-sm text-ruby-200">{errorMsg}</span>
                    </div>
                )}

                {status === 'success' && (
                    <div className="bg-emerald-500/10 border border-emerald-500/30 p-3 rounded-lg flex items-center gap-3 mt-6">
                        <div className="w-6 h-6 rounded-full bg-emerald-500 flex items-center justify-center shrink-0">
                            <Check className="w-4 h-4 text-black stroke-[3]" />
                        </div>
                        <span className="text-sm text-emerald-300 font-medium">Strategy settings saved. No structural changes detected — workspace remains healthy.</span>
                    </div>
                )}
            </div>

            <div className="mt-8 flex justify-end gap-3 z-10 border-t border-white/5 pt-4">
                <button 
                    onClick={() => setView('home')}
                    className="px-5 py-2.5 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-white/5 transition-all"
                >
                    Back to Project
                </button>
            </div>
        </div>
    );
}
