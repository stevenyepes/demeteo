import React, { useState, useEffect } from 'react';
import { useTauriEvent } from '../hooks/useTauriEvent';
import { Zap, Cpu, Play, Clock, ChevronRight, Settings, AlertTriangle, RotateCw, Check, Sliders, Terminal, Code } from 'lucide-react';
import { AgentTerminalDrawer } from './AgentTerminalDrawer';
import { invoke } from '@tauri-apps/api/core';
import { ConfigOptionValue } from '../types';
import { getAgentModels } from '../lib/agentModels';
import { formatError } from '../lib/errors';
import { useErrorBus } from '../lib/errorBus';
import { saveProjectSettings } from '../lib/project';
import { TerminalWindow } from './TerminalWindow';

export const MOCK_FEATURES = [
    {
        id: 'f-8a7b9c',
        title: 'Migrate Session Auth to JWT Tokens',
        status: 'gated',
        totalCost: 1.42,
        duration: '14m 22s',
        steps: [
            { id: 's1', type: 'agent', title: 'Research Codebase', agent: 'claude-sys-1', status: 'done', cost: 0.15, time: '2m 10s' },
            { id: 's2', type: 'agent', title: 'Draft Implementation Spec', agent: 'claude-sys-1', status: 'done', cost: 0.35, time: '4m 05s' },
            {
                id: 's3', type: 'parallel', title: 'Generate Utility Stubs', status: 'done', cost: 0.42, time: '3m 12s', subtasks: [
                    { title: 'jwt_encoder.ts', agent: 'opencode-alpha', status: 'done' },
                    { title: 'jwt_decoder.ts', agent: 'opencode-beta', status: 'done' },
                ]
            },
            { id: 's4', type: 'gate', title: 'Review Security Middleware', status: 'waiting', cost: 0.00, time: 'Paused', requires: 'Human Approval' },
            { id: 's5', type: 'agent', title: 'Rewrite Integration Tests', agent: 'hermes-worker', status: 'pending', cost: 0.00, time: '--' },
        ]
    },
    {
        id: 'f-2b3c4d',
        title: 'Implement Redis Rate Limiting API',
        status: 'running',
        totalCost: 0.85,
        duration: '5m 10s',
        steps: [
            { id: 's1', type: 'agent', title: 'Analyze Endpoint Traffic', agent: 'claude-sys-1', status: 'done', cost: 0.10, time: '1m 00s' },
            { id: 's2', type: 'agent', title: 'Write Redis Lua Scripts', agent: 'opencode-alpha', status: 'running', cost: 0.75, time: '4m 10s' },
        ]
    }
];

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

interface Feature {
  id: string;
  project_id: string;
  title: string;
  status: string;
  total_cost: number;
  tokens?: number | null;
  duration: string;
  created_at: number;
  agent_kind?: string | null;
  model?: string | null;
}

const formatTokens = (tokens: number | null | undefined): string => {
  if (tokens == null) return '0';
  if (tokens >= 1_000_000) {
    return `${(tokens / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  }
  if (tokens >= 1_000) {
    return `${(tokens / 1_000).toFixed(1).replace(/\.0$/, '')}k`;
  }
  return tokens.toString();
};

interface ProjectHomeProps {
  setView: (view: string) => void;
  activeProject: Project;
  setActiveFeatureId: (id: string) => void;
  setActiveFeatureTitle?: (title: string) => void;
  setProjects?: React.Dispatch<React.SetStateAction<Project[]>>;
  sidebarCollapsed?: boolean;
}

const ProjectHome: React.FC<ProjectHomeProps> = ({ setView, activeProject, setActiveFeatureId, setActiveFeatureTitle, setProjects, sidebarCollapsed }) => {
    const { reportError } = useErrorBus();
    const [featureInput, setFeatureInput] = useState('');
    const [isExpanded, setIsExpanded] = useState(false);
    const [features, setFeatures] = useState<any[]>([]);
    const [isLoadingFeatures, setIsLoadingFeatures] = useState(true);
    const [activeTab, setActiveTab] = useState<'pipelines' | 'terminal'>('pipelines');
    const [activeRepoPath, setActiveRepoPath] = useState<string>('');
    const [agentDrawerOpen, setAgentDrawerOpen] = useState(false);

    useEffect(() => {
        setActiveTab('pipelines');
    }, [activeProject.id]);

    useTauriEvent<{ feature_id: string; status: string }>('feature_status_changed', ({ feature_id, status }) => {
        setFeatures(prev => prev.map(f => f.id === feature_id ? { ...f, status } : f));
    });

    // Repositories and Workflows integration
    const [repositories, setRepositories] = useState<any[]>([]);
    const [selectedRepos, setSelectedRepos] = useState<string[]>([]);
    const [workflows, setWorkflows] = useState<any[]>([]);
    const [selectedWorkflow, setSelectedWorkflow] = useState<any | null>(null);
    const [userOverriddenWorkflow, setUserOverriddenWorkflow] = useState(false);
    const [isCustomizeOpen, setIsCustomizeOpen] = useState(false);

    // Coding Agent & Model Selector States
    const [agentConfigs, setAgentConfigs] = useState<any[]>([]);
    const [selectedAgentKind, setSelectedAgentKind] = useState<string>('');
    const [selectedModel, setSelectedModel] = useState<string>('');
    const [availableModels, setAvailableModels] = useState<ConfigOptionValue[]>([]);
    const [isLoadingModels, setIsLoadingModels] = useState(false);
    const [isDelegating, setIsDelegating] = useState(false);

    // Defaults from settings
    const [defaultAgentKind, setDefaultAgentKind] = useState<string>('');
    const [defaultModel, setDefaultModel] = useState<string>('');

    // Fetch models on selected agent change
    useEffect(() => {
        let cancelled = false;
        const fetchModels = async () => {
            if (!selectedAgentKind) {
                setAvailableModels([]);
                return;
            }
            setIsLoadingModels(true);
            try {
                const machineId = activeProject.compute_type === 'remote' ? activeProject.remote_host || 'local' : 'local';
                const models = await getAgentModels(machineId, selectedAgentKind);
                if (!cancelled) setAvailableModels(models);
            } catch (err) {
                if (!cancelled) {
                    console.warn("Failed to fetch models for agent:", selectedAgentKind, err);
                    setAvailableModels([]);
                }
            } finally {
                if (!cancelled) setIsLoadingModels(false);
            }
        };
        fetchModels();
        return () => { cancelled = true; };
    }, [selectedAgentKind, activeProject.compute_type, activeProject.remote_host]);

    // Retry and recovery states
    const [localBootstrapStep, setLocalBootstrapStep] = useState<'idle' | 'bootstrapping' | 'strategy_proposal' | 'error'>('idle');
    const [bootstrapError, setBootstrapError] = useState('');

    // Strategy Form States
    const [defaultBranch, setDefaultBranch] = useState('main');
    const [branchPrefix, setBranchPrefix] = useState('demeteo/features/');
    const [testCommand, setTestCommand] = useState('');
    const [prTemplate, setPrTemplate] = useState('');
    const [conflictPolicy, setConflictPolicy] = useState('always_gate');
    const [featureLifecycle, setFeatureLifecycle] = useState('archive');


    const handleRetryBootstrap = async () => {
        setLocalBootstrapStep('bootstrapping');
        setBootstrapError('');
        try {
            // Read existing settings so we preserve user-customized values
            const existing = await invoke<any>('get_proposed_strategy', {
                projectId: activeProject.id
            });

            const strategy = await invoke<any>('bootstrap_project', {
                projectId: activeProject.id
            });

            const ext = existing?.worktree_strategy;
            setDefaultBranch(ext?.default_branch ?? strategy.default_branch);
            setBranchPrefix(ext?.branch_prefix ?? strategy.branch_prefix);
            setTestCommand(ext?.test_command ?? strategy.test_command ?? '');
            setPrTemplate(ext?.pr_template ?? strategy.pr_template ?? '');
            setLocalBootstrapStep('strategy_proposal');
        } catch (err: any) {
            setLocalBootstrapStep('error');
            setBootstrapError(formatError(err));
        }
    };

    const handleApproveStrategy = async () => {
        try {
            // Utility merges with existing DB values, so we only pass the
            // fields shown in this simple form. Everything else is preserved.
            await saveProjectSettings(activeProject.id, {
                default_branch: defaultBranch,
                branch_prefix: branchPrefix,
                test_command: testCommand || null,
                pr_template: prTemplate || null,
                conflict_policy: conflictPolicy,
                feature_lifecycle: featureLifecycle,
            });

            // Update parent projects status to 'idle'
            if (setProjects) {
                setProjects(prev => prev.map(p => p.id === activeProject.id ? { ...p, status: 'idle' } : p));
            }
            setLocalBootstrapStep('idle');
        } catch (err: any) {
            setLocalBootstrapStep('error');
            setBootstrapError(formatError(err));
        }
    };

    useEffect(() => {
        const fetchWorkspaceData = async () => {
            setIsLoadingFeatures(true);
            const machineId = activeProject.compute_type === 'remote' ? activeProject.remote_host || 'local' : 'local';

            const [featuresRes, reposRes, workflowsRes, settingsRes, configsRes] = await Promise.allSettled([
                invoke<Feature[]>('fetch_active_features', { projectId: activeProject.id }),
                invoke<any[]>('get_repositories_for_project', { projectId: activeProject.id }),
                invoke<any[]>('workflow_list'),
                invoke<any>('get_proposed_strategy', { projectId: activeProject.id }),
                invoke<any[]>('get_agent_configs', { machineId })
            ]);

            // Handle active features
            if (featuresRes.status === 'fulfilled' && featuresRes.value) {
                const res = featuresRes.value;
                if (res && res.length > 0) {
                    const mapped = res.map(f => ({
                        id: f.id,
                        title: f.title,
                        status: f.status,
                        totalCost: f.total_cost,
                        tokens: f.tokens || 0,
                        duration: f.duration,
                        steps: []
                    }));
                    setFeatures(mapped);
                } else {
                    setFeatures([]);
                }
            } else {
                if (featuresRes.status === 'rejected') {
                    console.error("Failed to fetch active features:", featuresRes.reason);
                }
                setFeatures([]);
            }
            setIsLoadingFeatures(false);

            // Handle repositories
            if (reposRes.status === 'fulfilled' && reposRes.value) {
                const mapped = reposRes.value.map(r => ({
                    path: r.repo_path,
                    name: r.repo_path.split('/').pop() || r.repo_path
                }));
                setRepositories(mapped);
                if (mapped.length > 0) {
                    setActiveRepoPath(mapped[0].path);
                }
            } else if (reposRes.status === 'rejected') {
                console.error("Failed to fetch repositories:", reposRes.reason);
            }

            // Handle workflows
            if (workflowsRes.status === 'fulfilled' && workflowsRes.value) {
                const list = workflowsRes.value;
                setWorkflows(list);
                if (list.length > 0) {
                    setSelectedWorkflow(list[0]);
                }
            } else if (workflowsRes.status === 'rejected') {
                console.error("Failed to fetch workflows:", workflowsRes.reason);
            }

            // Handle proposed strategy settings
            if (settingsRes.status === 'fulfilled' && settingsRes.value) {
                const settings = settingsRes.value;
                if (settings) {
                    const defAgent = settings.default_agent_kind || '';
                    const defModel = settings.default_model || '';
                    setDefaultAgentKind(defAgent);
                    setDefaultModel(defModel);
                    setSelectedAgentKind(defAgent);
                    setSelectedModel(defModel);
                }
            } else if (settingsRes.status === 'rejected') {
                console.error("Failed to fetch project settings:", settingsRes.reason);
            }

            // Handle agent configs
            if (configsRes.status === 'fulfilled' && configsRes.value) {
                setAgentConfigs(configsRes.value || []);
            } else {
                if (configsRes.status === 'rejected') {
                    console.error("Failed to fetch agent configs:", configsRes.reason);
                }
                setAgentConfigs([]);
            }
        };
        fetchWorkspaceData();
    }, [activeProject.id]);

    useEffect(() => {
        if (!featureInput || userOverriddenWorkflow) return;
        
        const inputLower = featureInput.toLowerCase();
        let matchedWf = workflows[0];
        
        if (inputLower.includes('bug') || inputLower.includes('fix') || inputLower.includes('issue') || inputLower.includes('error') || inputLower.includes('crash')) {
            matchedWf = workflows.find(w => w.name.toLowerCase().includes('bug') || w.name.toLowerCase().includes('fix')) || matchedWf;
        } else if (inputLower.includes('refactor') || inputLower.includes('cleanup') || inputLower.includes('rewrite')) {
            matchedWf = workflows.find(w => w.name.toLowerCase().includes('refactor')) || matchedWf;
        } else if (inputLower.includes('doc') || inputLower.includes('readme') || inputLower.includes('comment')) {
            matchedWf = workflows.find(w => w.name.toLowerCase().includes('doc')) || matchedWf;
        } else if (inputLower.includes('experiment') || inputLower.includes('test') || inputLower.includes('prototype')) {
            matchedWf = workflows.find(w => w.name.toLowerCase().includes('experiment') || w.name.toLowerCase().includes('test')) || matchedWf;
        }
        
        if (matchedWf) {
            setSelectedWorkflow(matchedWf);
        }
    }, [featureInput, workflows, userOverriddenWorkflow]);

    const handleStartFeature = async () => {
        if (!featureInput) return;
        if (!selectedWorkflow) {
            reportError("Please select a workflow from the customization panel or build one first.", { kind: "validation" });
            return;
        }
        setIsDelegating(true);
        try {
            const res = await invoke<Feature>('start_feature', { 
                projectId: activeProject.id, 
                workflowId: selectedWorkflow.id,
                title: featureInput,
                description: featureInput,
                agentKind: selectedAgentKind || null,
                model: selectedModel || null
            });
            const newFeature = {
                id: res.id,
                title: res.title,
                status: res.status,
                totalCost: res.total_cost,
                tokens: res.tokens || 0,
                duration: res.duration,
                steps: []
            };
            setFeatures(prev => [newFeature, ...prev]);
            setActiveFeatureId(res.id);
            if (setActiveFeatureTitle) {
                setActiveFeatureTitle(res.title);
            }
            setView('detail');
        } catch (err) {
            console.error("Failed to start feature pipeline:", err);
            reportError(err);
        } finally {
            setIsDelegating(false);
        }
    };

    const isCurrentlyFailed = activeProject.status === 'error';
    const isCurrentlyBootstrapping = activeProject.status === 'bootstrapping';

    const currentStep = localBootstrapStep !== 'idle' ? localBootstrapStep : 
                        isCurrentlyFailed ? 'error' : 
                        isCurrentlyBootstrapping ? 'bootstrapping' : 'idle';

    if (currentStep === 'bootstrapping') {
        return (
            <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
                <div className="absolute top-1/4 left-1/2 -translate-x-1/2 w-[600px] h-[300px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none"></div>
                <div className="glass-panel max-w-lg w-full p-8 rounded-xl flex flex-col items-center text-center relative border border-white/10 shadow-2xl">
                    <RotateCw className="w-12 h-12 text-cyan-400 animate-spin mb-6" />
                    <h2 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Bootstrap In Progress</h2>
                    <p className="text-sm text-slate-400 mb-6 leading-relaxed">
                        Demeteo is securely checking out your repositories and running structural analysis.
                    </p>
                    <div className="w-full bg-black/40 border border-white/5 rounded-lg p-4 font-mono text-left text-xs space-y-2.5 text-slate-300">
                        <div className="flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse"></span>
                            <span>Resolving Provider Credentials...</span>
                        </div>
                        <div className="flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse"></span>
                            <span>Cloning Git Repositories...</span>
                        </div>
                        <div className="flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-slate-600"></span>
                            <span className="text-slate-500">Detecting project workflow patterns...</span>
                        </div>
                    </div>
                </div>
            </div>
        );
    }

    if (currentStep === 'error') {
        return (
            <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
                <div className="glass-panel max-w-lg w-full p-8 rounded-xl flex flex-col items-center text-center relative border border-ruby-500/20 shadow-2xl">
                    <AlertTriangle className="w-12 h-12 text-ruby-400 mb-4 animate-pulse" />
                    <h2 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Bootstrap Failed</h2>
                    <p className="text-sm text-slate-400 mb-6 leading-relaxed">
                        Demeteo could not clone configured repositories or analyze workspace structures. Verify target compute availability, credentials, and mapped repository paths.
                    </p>
                    {bootstrapError && (
                        <div className="w-full bg-black/40 border border-ruby-500/10 rounded-lg p-4 font-mono text-left text-xs text-ruby-300 overflow-x-auto mb-6 max-h-[150px]">
                            {bootstrapError}
                        </div>
                    )}
                    <div className="flex gap-3">
                        <button onClick={() => setView('project-settings')} className="px-5 py-2.5 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all flex items-center gap-1.5 font-medium">
                            <Settings className="w-4 h-4" /> Configure Workspace
                        </button>
                        <button onClick={handleRetryBootstrap} className="px-5 py-2.5 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg transition-all font-semibold shadow-[0_0_15px_rgba(239,68,68,0.3)] flex items-center gap-1.5">
                            <RotateCw className="w-4 h-4 animate-pulse" /> Retry Bootstrap
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    if (currentStep === 'strategy_proposal') {
        return (
            <div className="flex-1 overflow-y-auto p-8 relative flex items-center justify-center bg-[#08090c]">
                <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[400px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none"></div>
                <div className="glass-panel max-w-xl w-full p-6 rounded-xl flex flex-col border-white/10 shadow-2xl text-left">
                    <div className="mb-6 border-b border-white/5 pb-4">
                        <h3 className="font-outfit font-semibold text-cyan-400 uppercase tracking-widest text-xs mb-1">STRATEGY DETECTED</h3>
                        <h2 className="text-xl font-bold text-white">Configure Worktree Strategy</h2>
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
                        <button onClick={() => setLocalBootstrapStep('idle')} className="px-5 py-2.5 text-sm font-medium text-slate-400 hover:text-white transition-colors">Cancel</button>
                        <button onClick={handleApproveStrategy} className="px-6 py-2.5 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg shadow-[0_0_15px_rgba(16,185,129,0.3)] transition-all flex items-center gap-2">
                            <Check className="w-4 h-4" /> Approve & Build Workspace
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="flex-1 flex flex-col p-8 relative overflow-hidden bg-[#0a0c10]">
            <div className="max-w-7xl mx-auto w-full flex-1 flex flex-col min-h-0 space-y-6">

                {/* Header Block with Telemetry */}
                <div className="flex justify-between items-end shrink-0">
                    <div>
                        <div className="flex items-center gap-2 mb-2">
                            <h1 className="text-3xl font-outfit font-bold text-white tracking-tight">{activeProject.name}</h1>
                            <button
                                onClick={() => setView('project-settings')}
                                className="p-1.5 text-slate-400 hover:text-white rounded-md hover:bg-white/5 transition-all"
                                title="Workspace Settings"
                            >
                                <Settings className="w-5 h-5" />
                            </button>
                        </div>
                        <p className="text-sm text-slate-400">Connected via GitHub Enterprise &bull; Default Workflow: Standard Feature Pipeline</p>
                    </div>
                    <div className="glass-panel px-4 py-2 rounded-lg flex gap-4 text-xs font-mono">
                        <div className="flex flex-col"><span className="text-slate-500">Fleet Active</span><span className="text-emerald-400 font-bold">{features.filter(f => f.status === 'running').length} Nodes</span></div>
                        <div className="w-px bg-white/10"></div>
                        <div className="flex flex-col"><span className="text-slate-500">Token Spend</span><span className="text-white">{formatTokens(activeProject.tokens)}</span></div>
                    </div>
                </div>

                {/* Tabs Selector */}
                {activeProject.compute_type === 'remote' && (
                    <div className="tabs-bar shrink-0">
                        <button
                            onClick={() => setActiveTab('pipelines')}
                            className={`tab ${activeTab === 'pipelines' ? 'active' : ''}`}
                        >
                            <Sliders className="w-3.5 h-3.5" />
                            <span>Pipelines</span>
                        </button>
                        <button
                            onClick={() => setActiveTab('terminal')}
                            className={`tab ${activeTab === 'terminal' ? 'active' : ''}`}
                        >
                            <Terminal className="w-3.5 h-3.5" />
                            <span>Terminal</span>
                        </button>
                    </div>
                )}

                {activeTab === 'pipelines' || activeProject.compute_type !== 'remote' ? (
                    <div className="flex-1 overflow-y-auto space-y-8 pr-1 min-h-0">
                        {/* Start Feature Expanded Card */}
                <div className={`glass-panel rounded-2xl overflow-hidden transition-[padding] duration-300 ${isExpanded ? 'p-6' : 'p-2 relative group'}`}>
                    {!isExpanded && (
                        <div className="absolute inset-0 bg-gradient-to-r from-violet-500/10 to-cyan-500/10 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                    )}

                    <div className="relative flex items-start gap-4">
                        <div className={`mt-2 ml-2 rounded-full flex items-center justify-center transition-colors ${isExpanded ? 'bg-violet-500/20 text-violet-400 p-2' : 'text-slate-500'}`}>
                            <Zap className="w-5 h-5" />
                        </div>
                        <div className="flex-1 min-w-0">
                            {isExpanded ? (
                                <>
                                    <h3 className="font-outfit text-white font-medium mb-4">Start a new Feature Pipeline</h3>
                                    <div className="w-full mb-4">
                                        <textarea
                                            autoFocus
                                            value={featureInput}
                                            onChange={(e) => setFeatureInput(e.target.value)}
                                            placeholder="Describe the feature or bug you want the agent fleet to build... (e.g., 'Configure OAuth2 with Google authentication')"
                                            className="w-full block bg-black/20 border border-white/5 rounded-lg p-4 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-violet-500/50 focus:ring-1 focus:ring-violet-500/50 min-h-[120px] transition-colors duration-200 resize-none"
                                        />
                                        
                                        {/* Active Executor Configuration Summary (Micro-telemetry) */}
                                        {featureInput.length > 0 && (
                                            <div className="mt-2 flex items-center gap-2 text-[11px] text-slate-400 font-mono pl-1">
                                                <Zap className="w-3.5 h-3.5 text-violet-400" />
                                                <span>Pipeline Agent:</span>
                                                <span className="text-white font-semibold">
                                                    {selectedAgentKind ? selectedAgentKind.replace(/-/g, ' ') : (defaultAgentKind ? `${defaultAgentKind.replace(/-/g, ' ')} (default)` : 'Auto (Workflow)')}
                                                </span>
                                                <span className="text-slate-600">•</span>
                                                <span>Model:</span>
                                                <span className="text-cyan-400 font-semibold">
                                                    {selectedModel ? selectedModel : (defaultModel ? `${defaultModel} (default)` : 'Default')}
                                                </span>
                                            </div>
                                        )}
                                    </div>



                                    {/* Real-time Keyword Auto-Inference & Suggested Repos */}
                                    {featureInput.length > 3 && (
                                        <div className="mt-4 p-3.5 border border-cyan-500/20 bg-cyan-500/5 rounded-xl flex flex-col gap-3 relative overflow-hidden">
                                            <div className="absolute top-0 right-0 w-32 h-32 bg-cyan-500/5 rounded-full blur-xl pointer-events-none"></div>
                                            <div className="flex items-center gap-2 text-xs font-semibold text-cyan-300">
                                                <Cpu className="w-4 h-4 animate-pulse" />
                                                <span>Pipeline Smart Inference</span>
                                            </div>

                                            {/* Highlighted Workflow */}
                                            {selectedWorkflow && (
                                                <div className="text-xs text-slate-300 flex items-center gap-1.5 flex-wrap">
                                                    <span>Workflow matched:</span>
                                                    <span className="px-2 py-0.5 rounded bg-violet-500/20 border border-violet-500/30 text-violet-300 font-medium font-outfit">
                                                        {selectedWorkflow.name}
                                                    </span>
                                                    {selectedWorkflow.schedule && (
                                                        <span className="px-2 py-0.5 rounded bg-violet-500/25 border border-violet-500/40 text-violet-300 text-[10px] font-medium font-outfit flex items-center gap-1">
                                                            <Clock className="w-3 h-3 text-violet-400" /> Scheduled: {selectedWorkflow.schedule.cron}
                                                        </span>
                                                    )}
                                                    {userOverriddenWorkflow && (
                                                        <span className="text-[10px] text-slate-500">(custom override)</span>
                                                    )}
                                                </div>
                                            )}

                                            {/* Suggested Repositories Chips */}
                                            <div className="text-xs text-slate-300 space-y-1.5">
                                                <div>Suggested Repository Scope:</div>
                                                <div className="flex flex-wrap gap-2">
                                                    {repositories.map(repo => {
                                                        const isSuggested = featureInput.toLowerCase().includes(repo.name.toLowerCase()) ||
                                                                            featureInput.toLowerCase().includes(repo.path.split('/').pop().toLowerCase());
                                                        const isSelected = selectedRepos.includes(repo.path);

                                                        return (
                                                            <button
                                                                type="button"
                                                                key={repo.path}
                                                                onClick={() => {
                                                                    setSelectedRepos(prev => 
                                                                        prev.includes(repo.path)
                                                                            ? prev.filter(r => r !== repo.path)
                                                                            : [...prev, repo.path]
                                                                    );
                                                                }}
                                                                className={`px-3 py-1 rounded-full text-xs font-mono transition-all border ${
                                                                    isSelected 
                                                                        ? 'bg-cyan-500/20 border-cyan-400 text-cyan-300 shadow-[0_0_10px_rgba(6,182,212,0.15)]'
                                                                        : isSuggested
                                                                            ? 'bg-violet-500/10 border-violet-500/30 text-violet-300 animate-pulse'
                                                                            : 'bg-black/40 border-white/5 text-slate-500 hover:border-white/10'
                                                                }`}
                                                            >
                                                                {repo.path}
                                                                {isSuggested && (
                                                                    <span className="ml-1 text-[9px] px-1 rounded bg-violet-500/20 text-violet-300">Suggested</span>
                                                                )}
                                                            </button>
                                                        );
                                                    })}
                                                </div>
                                            </div>
                                        </div>
                                    )}

                                    {/* Customize panel drawer */}
                                    {isCustomizeOpen && (
                                        <div className="mt-4 p-4 border border-white/5 bg-black/45 rounded-xl space-y-4 animate-fadeIn">
                                            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                                                {/* Workflow Select */}
                                                <div>
                                                    <label className="block text-[10px] font-mono text-slate-500 mb-1.5 uppercase tracking-wider">Orchestration Workflow</label>
                                                    <select
                                                        value={selectedWorkflow?.id || ''}
                                                        onChange={e => {
                                                            const wf = workflows.find(w => w.id === e.target.value);
                                                            if (wf) {
                                                                setSelectedWorkflow(wf);
                                                                setUserOverriddenWorkflow(true);
                                                            }
                                                        }}
                                                        className="w-full bg-[#08090c] border border-white/10 rounded-lg p-2 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                                                    >
                                                        {workflows.map(wf => (
                                                            <option key={wf.id} value={wf.id}>
                                                              {wf.name} ({wf.is_starter ? 'Starter' : 'Custom'}){wf.schedule ? ' ⏰' : ''}
                                                            </option>
                                                        ))}
                                                    </select>
                                                </div>

                                                {/* Repositories checkboxes */}
                                                <div>
                                                    <label className="block text-[10px] font-mono text-slate-500 mb-1.5 uppercase tracking-wider">Target Repositories</label>
                                                    <div className="text-xs text-slate-300 font-mono p-2 bg-[#08090c] border border-white/10 rounded-lg max-h-[85px] overflow-y-auto space-y-1">
                                                        {repositories.length === 0 ? (
                                                            <span className="text-slate-500 italic">No repositories mapped.</span>
                                                        ) : (
                                                            repositories.map(repo => {
                                                                const isSelected = selectedRepos.includes(repo.path);
                                                                return (
                                                                    <label key={repo.path} className="flex items-center gap-2 cursor-pointer hover:text-white select-none">
                                                                        <input
                                                                            type="checkbox"
                                                                            checked={isSelected}
                                                                            onChange={() => {
                                                                                setSelectedRepos(prev =>
                                                                                    prev.includes(repo.path)
                                                                                        ? prev.filter(r => r !== repo.path)
                                                                                        : [...prev, repo.path]
                                                                                );
                                                                            }}
                                                                            className="rounded border-slate-600 bg-transparent text-cyan-500 focus:ring-0 w-3 h-3 cursor-pointer"
                                                                        />
                                                                        <span>{repo.path}</span>
                                                                    </label>
                                                                );
                                                            })
                                                        )}
                                                    </div>
                                                </div>

                                                {/* AI Agent Override */}
                                                <div>
                                                    <label className="block text-[10px] font-mono text-slate-500 mb-1.5 uppercase tracking-wider">
                                                        AI Agent Override {defaultAgentKind && `(Default: ${defaultAgentKind})`}
                                                    </label>
                                                    <div className="flex flex-wrap gap-2">
                                                        <button
                                                            type="button"
                                                            onClick={() => {
                                                                setSelectedAgentKind('');
                                                                setSelectedModel('');
                                                            }}
                                                            className={`px-3 py-1.5 rounded-lg border text-xs font-medium transition-all ${
                                                                !selectedAgentKind
                                                                    ? 'bg-violet-500/10 border-violet-500/50 text-violet-300'
                                                                    : 'bg-black/40 border-white/5 text-slate-400 hover:border-white/10'
                                                            }`}
                                                        >
                                                            Use Default
                                                        </button>
                                                        {agentConfigs.filter(a => a.enabled && a.kind !== 'antigravity').map(agent => (
                                                            <button
                                                                key={agent.kind}
                                                                type="button"
                                                                onClick={() => {
                                                                    setSelectedAgentKind(agent.kind);
                                                                    setSelectedModel('');
                                                                }}
                                                                className={`px-3 py-1.5 rounded-lg border text-xs font-medium transition-all capitalize ${
                                                                    selectedAgentKind === agent.kind
                                                                        ? 'bg-violet-500/10 border-violet-500/50 text-violet-300'
                                                                        : 'bg-black/40 border-white/5 text-slate-400 hover:border-white/10'
                                                                }`}
                                                            >
                                                                {agent.kind.replace(/-/g, ' ')}
                                                            </button>
                                                        ))}
                                                    </div>
                                                </div>

                                                {/* Model Selection Override */}
                                                <div>
                                                    <label className="block text-[10px] font-mono text-slate-500 mb-1.5 uppercase tracking-wider">
                                                        LLM Model Override {defaultModel && `(Default: ${defaultModel})`}
                                                    </label>
                                                    {isLoadingModels ? (
                                                        <div className="w-full bg-[#08090c]/40 border border-white/10 rounded-lg p-2 text-xs text-slate-400 flex items-center gap-2">
                                                            <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                                                            <span>Probing models...</span>
                                                        </div>
                                                    ) : (
                                                        <div className="flex gap-2">
                                                            <select
                                                                value={selectedModel}
                                                                onChange={e => setSelectedModel(e.target.value)}
                                                                className="flex-1 min-w-0 bg-[#08090c] border border-white/10 rounded-lg p-2 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                                                                disabled={!selectedAgentKind}
                                                            >
                                                                <option value="">Use Default Model</option>
                                                                {availableModels.map(m => (
                                                                    <option key={m.value} value={m.value}>{m.name}</option>
                                                                ))}
                                                                {selectedModel && !availableModels.some(m => m.value === selectedModel) && (
                                                                    <option value={selectedModel}>{selectedModel} (custom)</option>
                                                                )}
                                                            </select>
                                                            <input
                                                                type="text"
                                                                value={selectedModel}
                                                                onChange={e => setSelectedModel(e.target.value)}
                                                                placeholder="Custom override"
                                                                className="w-1/3 shrink-0 min-w-[120px] bg-black/40 border border-white/10 rounded-lg p-1.5 text-xs text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600"
                                                                disabled={!selectedAgentKind}
                                                            />
                                                        </div>
                                                    )}
                                                </div>
                                            </div>

                                            {/* Pre-flight Step Timeline Preview */}
                                            {selectedWorkflow && (
                                                <div className="space-y-2 border-t border-white/5 pt-3">
                                                    <h4 className="text-[10px] font-mono text-slate-500 uppercase tracking-wider">Workflow Step Timeline Preview</h4>
                                                    <div className="flex flex-col md:flex-row md:items-center gap-3 md:gap-2 overflow-x-auto pb-2 w-full max-w-full">
                                                        {selectedWorkflow.steps && selectedWorkflow.steps.length > 0 ? (
                                                            selectedWorkflow.steps.map((step: any, idx: number) => (
                                                                <React.Fragment key={step.id}>
                                                                    <div className="flex items-center gap-2.5 p-2.5 rounded bg-black/40 border border-white/5 min-w-[150px] max-w-[200px] shrink-0">
                                                                        <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold ${
                                                                            step.kind === 'gate'
                                                                                ? 'bg-violet-500/20 text-violet-400 border border-violet-500/30'
                                                                                : step.kind === 'parallel'
                                                                                    ? 'bg-cyan-500/20 text-cyan-400 border border-cyan-500/30'
                                                                                    : 'bg-emerald-500/20 text-emerald-400 border border-emerald-500/30'
                                                                        }`}>
                                                                            {idx + 1}
                                                                        </div>
                                                                        <div className="min-w-0">
                                                                            <div className="text-xs text-white font-medium truncate">{step.title}</div>
                                                                            <div className="text-[9px] text-slate-500 capitalize">{step.kind}</div>
                                                                        </div>
                                                                    </div>
                                                                    {idx < selectedWorkflow.steps.length - 1 && (
                                                                        <ChevronRight className="w-3.5 h-3.5 text-slate-600 shrink-0 hidden md:block" />
                                                                    )}
                                                                </React.Fragment>
                                                            ))
                                                        ) : (
                                                            <span className="text-xs text-slate-500 italic">No steps defined in this workflow.</span>
                                                        )}
                                                    </div>
                                                </div>
                                            )}
                                        </div>
                                    )}

                                    <div className="mt-4 flex justify-between items-center">
                                        <div className="flex gap-4 items-center">
                                            <button
                                                type="button"
                                                onClick={() => setIsCustomizeOpen(!isCustomizeOpen)}
                                                className="text-xs font-semibold text-slate-400 hover:text-white flex items-center gap-1.5 transition-colors"
                                            >
                                                <Settings className="w-3.5 h-3.5" />
                                                {isCustomizeOpen ? 'Hide Customization' : 'Customize...'}
                                            </button>
                                        </div>
                                        <div className="flex gap-3">
                                            <button onClick={() => setIsExpanded(false)} className="px-4 py-2 text-sm font-medium text-slate-400 hover:text-white transition-colors">Cancel</button>
                                            <button
                                                onClick={handleStartFeature}
                                                disabled={isDelegating}
                                                className="px-6 py-2 text-sm font-medium bg-violet-600 hover:bg-violet-500 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded-md shadow-[0_0_15px_rgba(139,92,246,0.4)] transition-all flex items-center gap-2"
                                            >
                                                {isDelegating ? <RotateCw className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4" />} Delegate Workspace
                                            </button>
                                        </div>
                                    </div>

                                </>
                            ) : (
                                <input
                                    type="text"
                                    onClick={() => setIsExpanded(true)}
                                    placeholder="Draft and delegate a new feature pipeline..."
                                    className="w-full bg-transparent border-none p-2 text-sm text-white placeholder-slate-500 focus:outline-none cursor-pointer"
                                    readOnly
                                />
                            )}
                        </div>
                    </div>
                </div>

                {/* Code with Agent card */}
                <button
                  onClick={() => setAgentDrawerOpen(true)}
                  disabled={!activeRepoPath}
                  className="glass-panel rounded-2xl p-4 flex items-center gap-4 text-left w-full group hover:border-cyan-500/20 transition-all duration-300 disabled:opacity-40 disabled:cursor-not-allowed border border-white/5 hover:bg-cyan-500/5"
                >
                  <div className="w-9 h-9 rounded-full bg-cyan-500/10 border border-cyan-500/20 flex items-center justify-center shrink-0 group-hover:bg-cyan-500/20 transition-colors">
                    <Code className="w-4 h-4 text-cyan-400" />
                  </div>
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-white">Start a coding session</p>
                    <p className="text-xs text-slate-500 mt-0.5">
                      Open an interactive agent (Claude, OpenCode…) directly in the repo — no pipeline required
                    </p>
                  </div>
                  <Terminal className="w-4 h-4 text-slate-600 group-hover:text-cyan-400 transition-colors ml-auto shrink-0" />
                </button>

                {/* Active Features Tracking List */}
                <div>
                    <h2 className="font-outfit text-sm font-semibold text-slate-400 uppercase tracking-widest mb-4">Active Running Pipelines</h2>
                    <div className="space-y-4">
                        {isLoadingFeatures ? (
                            <div className="flex items-center justify-center p-8">
                                <RotateCw className="w-6 h-6 text-cyan-400 animate-spin" />
                            </div>
                        ) : features.length === 0 ? (
                            <div className="glass-panel p-8 rounded-2xl border border-white/5 text-center bg-black/20 flex flex-col items-center justify-center space-y-4 relative overflow-hidden">
                                <div className="absolute -top-10 -left-10 w-40 h-40 bg-violet-600/5 rounded-full blur-2xl pointer-events-none"></div>
                                <div className="absolute -bottom-10 -right-10 w-40 h-40 bg-cyan-600/5 rounded-full blur-2xl pointer-events-none"></div>
                                <div className="w-12 h-12 rounded-full bg-violet-500/10 border border-violet-500/25 flex items-center justify-center text-violet-400 mb-2">
                                    <Cpu className="w-6 h-6 animate-pulse" />
                                </div>
                                <h3 className="font-outfit text-white font-medium text-base">No active feature pipelines</h3>
                                <p className="text-xs text-slate-400 max-w-sm mx-auto leading-relaxed">
                                    There are no agent orchestration workflows running in this workspace right now. Use the tool above to start a new pipeline.
                                </p>
                            </div>
                        ) : (
                            features.map((feature: any) => (
                                <div
                                    key={feature.id}
                                    onClick={() => {
                                        setActiveFeatureId(feature.id);
                                        if (setActiveFeatureTitle) setActiveFeatureTitle(feature.title);
                                        setView('detail');
                                    }}
                                    className="glass-panel glass-panel-hover rounded-xl p-5 cursor-pointer relative overflow-hidden group"
                                >
                                    <div className={`absolute left-0 top-0 bottom-0 w-1 ${feature.status === 'gated' ? 'bg-violet-500 shadow-[0_0_10px_rgba(139,92,246,0.8)]' : 'bg-cyan-500 shadow-[0_0_10px_rgba(6,182,212,0.8)]'
                                        }`}></div>

                                    <div className="flex justify-between items-start gap-4">
                                        <div className="min-w-0 flex-1">
                                            <div className="flex items-center gap-3 mb-1">
                                                {feature.status === 'gated' ? (
                                                    <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-violet-500/10 border border-violet-500/20 text-violet-400 uppercase">GATED APPROVAL</span>
                                                ) : (
                                                    <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-cyan-500/10 border border-cyan-500/20 text-cyan-400 uppercase flex items-center gap-1">
                                                        <span className="w-1.5 h-1.5 rounded-full bg-cyan-400 animate-pulse"></span> RUNNING FLEET
                                                    </span>
                                                )}
                                                <span className="text-xs text-slate-500 font-mono truncate">{feature.id}</span>
                                            </div>
                                            <h3 className="text-lg font-outfit text-white line-clamp-2 break-words" title={feature.title}>{feature.title}</h3>
                                        </div>

                                        <div className="flex gap-6 text-right shrink-0 pt-1">
                                            <div>
                                                <div className="text-xs text-slate-500 font-mono flex items-center gap-1 justify-end"><Clock className="w-3 h-3" /> Duration</div>
                                                <div className="text-sm font-medium text-white">{feature.duration}</div>
                                            </div>
                                            <div>
                                                <div className="text-xs text-slate-500 font-mono flex items-center gap-1 justify-end"><Zap className="w-3 h-3 text-cyan-400 animate-pulse" /> Tokens</div>
                                                <div className="text-sm font-medium text-white">{formatTokens(feature.tokens || 0)}</div>
                                            </div>
                                            <ChevronRight className="w-5 h-5 text-slate-500 mt-2 opacity-0 group-hover:opacity-100 transition-opacity" />
                                        </div>
                                    </div>
                                </div>
                            ))
                        )}
                    </div>
                </div>
                </div>
                ) : (
                    <div className="flex-1 min-h-0 flex flex-col gap-4">
                        {repositories.length > 1 && (
                            <div className="flex items-center gap-2 text-xs font-mono text-slate-400 bg-white/5 border border-white/5 rounded-lg p-2.5 shrink-0">
                                <span>Select Repository:</span>
                                <select
                                    value={activeRepoPath}
                                    onChange={(e) => setActiveRepoPath(e.target.value)}
                                    className="bg-[#08090c] border border-white/10 rounded px-2.5 py-1 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                                >
                                    {repositories.map((repo) => (
                                        <option key={repo.path} value={repo.path}>
                                            {repo.path}
                                        </option>
                                    ))}
                                </select>
                            </div>
                        )}
                        <div className="flex-1 min-h-0">
                            {activeRepoPath ? (
                                <TerminalWindow
                                    projectId={activeProject.id}
                                    computeType={activeProject.compute_type || 'local'}
                                    remoteHost={activeProject.remote_host || null}
                                    repoPath={activeRepoPath}
                                />
                            ) : (
                                <div className="flex-1 flex items-center justify-center p-8 bg-[#050608] border border-white/5 rounded-xl">
                                    <span className="text-xs text-slate-500 italic">No repositories configured.</span>
                                </div>
                            )}
                        </div>
                    </div>
                )}

            </div>

        {activeRepoPath && (
          <AgentTerminalDrawer
            isOpen={agentDrawerOpen}
            onClose={() => setAgentDrawerOpen(false)}
            machineId={activeProject.compute_type === 'remote' ? activeProject.remote_host || 'local' : 'local'}
            repoPath={activeRepoPath}
            projectId={activeProject.id}
            computeType={activeProject.compute_type || 'local'}
            remoteHost={activeProject.remote_host || null}
            sidebarWidth={sidebarCollapsed ? 56 : 240}
          />
        )}
        </div>
    );
};

export default ProjectHome;
