import React, { useState, useEffect } from 'react';
import { 
    Settings, Save, Check, RotateCw, GitBranch, ShieldAlert, 
    Trash2, Box, Search, Plus, X, AlertTriangle, HardDrive, Server, Globe,
    Activity, RefreshCw, ChevronDown, ChevronUp, Zap, CircleAlert
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { ConfigOptionValue } from '../types';

interface Project {
    id: string;
    name: string;
    status: string;
    repos: number;
    nodes: number;
    spend: number;
    compute_type?: string;
    remote_host?: string | null;
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
}

interface ProjectSettings {
    project_id: string;
    worktree_strategy: WorktreeStrategy;
    conflict_policy: string;
    feature_lifecycle: string;
    default_agent_kind?: string | null;
    default_model?: string | null;
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
    const [activeTab, setActiveTab] = useState<'general' | 'strategy'>('general');
    const [status, setStatus] = useState<'idle' | 'saving' | 'success' | 'error'>('idle');
    const [errorMsg, setErrorMsg] = useState('');

    // General Configuration States
    const [projectName, setProjectName] = useState(activeProject.name);
    const [computeType, setComputeType] = useState(activeProject.compute_type || 'local');
    const [remoteHost, setRemoteHost] = useState(activeProject.remote_host || '');
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
    const [prTemplate, setPrTemplate] = useState('');
    const [conflictPolicy, setConflictPolicy] = useState('always_gate');
    const [featureLifecycle, setFeatureLifecycle] = useState('archive');

    // Warning Modals
    const [dirtyWarningRepos, setDirtyWarningRepos] = useState<RepoDirtyStatus[]>([]);
    const [pendingActionAfterConfirm, setPendingActionAfterConfirm] = useState<'save' | 'delete' | null>(null);
    const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

    // Coding Agent configs
    const [agentConfigs, setAgentConfigs] = useState<AgentConfigView[]>([]);

    // Default AI Executor States
    const [defaultAgentKind, setDefaultAgentKind] = useState<string>('');
    const [defaultModel, setDefaultModel] = useState<string>('');
    const [availableModelsForDefault, setAvailableModelsForDefault] = useState<ConfigOptionValue[]>([]);
    const [isLoadingModelsForDefault, setIsLoadingModelsForDefault] = useState(false);

    useEffect(() => {
        const fetchModels = async () => {
            if (!defaultAgentKind) {
                setAvailableModelsForDefault([]);
                return;
            }
            setIsLoadingModelsForDefault(true);
            try {
                const machineId = computeType === 'remote' ? remoteHost : 'local';
                const models = await invoke<ConfigOptionValue[]>('get_agent_models', {
                    machineId,
                    agentKind: defaultAgentKind
                });
                setAvailableModelsForDefault(models);
            } catch (err) {
                console.warn("Failed to fetch models for agent:", defaultAgentKind, err);
                setAvailableModelsForDefault([]);
            } finally {
                setIsLoadingModelsForDefault(false);
            }
        };
        fetchModels();
    }, [defaultAgentKind, computeType, remoteHost]);

    useEffect(() => {
        setConnectionStatus('idle');
    }, [remoteHost]);

    const fetchAgentConfigs = async () => {
        const machineId = computeType === 'remote' ? remoteHost : 'local';
        if (computeType === 'remote' && !remoteHost) {
            setAgentConfigs([]);
            return;
        }
        try {
            const configs = await invoke<AgentConfigView[]>('get_agent_configs', { machineId });
            setAgentConfigs(configs);
        } catch (err) {
            // The backend has no rows for this machine — that's a
            // legitimate state, not an error. Show an empty list and
            // let the user save new configs. Previously this catch
            // block silently replaced the error with mock data that
            // hardcoded `available: true`, which let the user start a
            // feature against an agent that was never installed.
            const message = err instanceof Error ? err.message : String(err);
            console.warn("No agent configs found for machine:", machineId, "—", message);
            setAgentConfigs([]);
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
            const message = err instanceof Error ? err.message : String(err);
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
                    setPrTemplate(res.worktree_strategy.pr_template || '');
                    setConflictPolicy(res.conflict_policy);
                    setFeatureLifecycle(res.feature_lifecycle);
                    setDefaultAgentKind(res.default_agent_kind || '');
                    setDefaultModel(res.default_model || '');
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
                const message = err instanceof Error ? err.message : String(err);
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
                    console.error(`Failed to fetch repos for provider ${p.name}:`, err);
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
            const message = err instanceof Error ? err.message : String(err);
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
            setErrorMsg("Connection test failed: " + String(err));
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
            console.error("Failed to check dirty status of repositories:", err);
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
                    agents: agentConfigs.map(a => ({ kind: a.kind, enabled: a.enabled }))
                });
            }
        } catch (err) {
            console.error("Failed to save agent configs:", err);
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
                            pr_template: prTemplate || null
                        },
                        conflict_policy: conflictPolicy,
                        feature_lifecycle: featureLifecycle,
                        default_agent_kind: defaultAgentKind || null,
                        default_model: defaultModel || null
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
                setErrorMsg(String(err));
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
            setPrTemplate(strategy.pr_template || '');
            setBootstrapStep('strategy_proposal');
        } catch (err: any) {
            setBootstrapStep('error');
            setBootstrapError(String(err));
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
                        pr_template: prTemplate || null
                    },
                    conflict_policy: conflictPolicy,
                    feature_lifecycle: featureLifecycle,
                    default_agent_kind: defaultAgentKind || null,
                    default_model: defaultModel || null
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
            setBootstrapError(String(err));
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
            setErrorMsg("Failed to delete workspace: " + String(err));
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
                                        <input 
                                            type="text" 
                                            value={remoteHost} 
                                            onChange={e => setRemoteHost(e.target.value)}
                                            placeholder="e.g. machine_id"
                                            className="flex-1 bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white font-mono focus:outline-none focus:border-cyan-500/50"
                                        />
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
                                            {isTestingConnection ? 'Testing...' : connectionStatus === 'success' ? 'Connected' : connectionStatus === 'error' ? 'Failed' : 'Test Connection'}
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
                ) : (
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
                                    {agentConfigs.filter(a => a.enabled).map(a => (
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
                            </div>
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
                                    onClick={fetchAgentConfigs}
                                    className="p-1 rounded text-slate-500 hover:text-cyan-400 hover:bg-white/5 transition-all"
                                    title="Re-check agent availability"
                                >
                                    <RotateCw className="w-3.5 h-3.5" />
                                </button>
                            </div>
                            <p className="text-xs text-slate-400">
                                Enable or disable specific AI coding agents for this workspace. Demeteo validates if these agents' CLI binaries are available on the selected compute server.
                            </p>

                            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-2">
                                {agentConfigs.length === 0 ? (
                                    <div className="md:col-span-2 text-xs text-slate-500 italic p-2">No agents found on target machine.</div>
                                ) : agentConfigs.map((agent) => (
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
