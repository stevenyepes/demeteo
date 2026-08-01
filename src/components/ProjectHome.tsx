import { useCallback, useEffect, useState } from 'react';
import { useTauriEvent } from '../hooks/useTauriEvent';
import { Zap, Cpu, Clock, ChevronRight, Settings, AlertTriangle, RotateCw, Check, Sliders, Terminal } from 'lucide-react';
import { Feature, Repository } from '../types';
import { formatTokens } from '../lib/utils';
import { formatError } from '../lib/errors';
import { getProposedStrategy, getRepositoriesForProject, saveProjectSettings } from '../lib/project';
import { bootstrapProject } from '../lib/createProjectWizard';
import { fetchActiveFeatures } from '../lib/features';
import { listMirroredRuns } from '../lib/remoteRuns';
import { listWorkflows } from '../lib/workflows';
import { AttachmentDropzone, type LaunchStageEntry } from './AttachmentDropzone';
import { StartSessionButton } from './StartSessionButton';
import { useNavigation, useProject, useUIState, useTerminalPanel } from '../context';
import { featureRunStatus, runStatusMeta, TONE_CHIP, type RunStatusTone } from '../lib/runStatus';
import { buildWorkflowById, classifyWorkflowBadge } from '../lib/workflowBadge';
import {
    extractClipboardImageFiles,
    recoverClipboardImageFile,
    stageBrowserFilesForLaunch,
} from '../lib/attachments';

/**
 * Left accent bar per tone. Local to this component (the way StatusBadge
 * keeps its own TONE_DOT) because the glow is specific to these cards —
 * the shared registry only carries the flat `TONE_BORDER_L` border.
 */
const TONE_ACCENT: Record<RunStatusTone, string> = {
    emerald: 'bg-emerald-500 shadow-[0_0_10px_rgba(16,185,129,0.8)]',
    cyan:    'bg-cyan-500 shadow-[0_0_10px_rgba(6,182,212,0.8)]',
    violet:  'bg-violet-500 shadow-[0_0_10px_rgba(139,92,246,0.8)]',
    amber:   'bg-amber-500 shadow-[0_0_10px_rgba(245,158,11,0.8)]',
    ruby:    'bg-ruby-500 shadow-[0_0_10px_rgba(239,68,68,0.8)]',
    slate:   'bg-slate-600 shadow-[0_0_10px_rgba(100,116,139,0.6)]',
};

const ProjectHome = () => {
    const { navigate } = useNavigation();
    const { state: { currentProjectId, projects }, dispatch: projDispatch } = useProject();
    const { uiDispatch } = useUIState();
    const activeProject = projects.find(p => p.id === currentProjectId)!;
    const [featureInput, setFeatureInput] = useState('');
    const [features, setFeatures] = useState<Feature[]>([]);
    const [isLoadingFeatures, setIsLoadingFeatures] = useState(true);
    const [activeTab, setActiveTab] = useState<'pipelines' | 'terminal'>('pipelines');
    const [activeRepositoryId, setActiveRepositoryId] = useState<string>('');
    // Feature ids that have a remote-run mirror → they execute detached under
    // the runner rather than on this machine. Drives the per-card transport
    // badge. Empty when nothing detached is (or was) tracked in this project.
    const [detachedIds, setDetachedIds] = useState<Set<string>>(new Set());

    useEffect(() => {
        setActiveTab('pipelines');
    }, [activeProject.id]);

    useTauriEvent<{ feature_id: string; status: string }>('feature_status_changed', ({ feature_id, status }) => {
        setFeatures(prev => prev.map(f => f.id === feature_id ? { ...f, status } : f));
    });

    // Repositories drive the terminal tab / coding-session target.
    const [repositories, setRepositories] = useState<Repository[]>([]);
    const [repositoriesProjectId, setRepositoriesProjectId] = useState<string | null>(null);
    // Workflow id → display meta, used to label the Active Pipelines list.
    const [workflowById, setWorkflowById] = useState<Map<string, { name: string; is_starter: boolean }>>(new Map());

    // Staged attachments for the inline composer. Handed to the Start
    // Feature modal as a seed on launch; see AttachmentDropzone.tsx for
    // the launch-stage contract.
    const [attachments, setAttachments] = useState<LaunchStageEntry[]>([]);
    const [attachmentError, setAttachmentError] = useState<string | null>(null);

    // The inline composer is a lightweight seed for the one launch
    // surface (Alternative A): it captures a title + attachments and
    // hands off to the Start Feature modal, which owns every launch
    // knob (repos, runner, overrides) and the actual start_feature call.
    const stageClipboardFiles = useCallback(async (files: File[]) => {
        try {
            setAttachments(await stageBrowserFilesForLaunch(files, attachments));
        } catch (err) {
            console.error('ProjectHome: failed to stage pasted attachment', err);
        }
    }, [attachments]);

    const handleComposerPaste = useCallback(
        async (e: React.ClipboardEvent<HTMLDivElement>) => {
            const extraction = extractClipboardImageFiles(e.clipboardData);
            if (extraction.kind === 'none') {
                if (e.clipboardData.items.length !== 0) return;
                const recovery = await recoverClipboardImageFile();
                if (recovery.kind !== 'recovered') {
                    setAttachmentError('This webview could not read image bytes from the clipboard. Save it and attach it, or try another clipboard source.');
                    return;
                }
                e.preventDefault();
                await stageClipboardFiles([recovery.file]);
                setAttachmentError(null);
                return;
            }
            if (extraction.kind === 'unavailable') {
                setAttachmentError('The clipboard offered an image, but this webview could not access its file. Save it and attach it, or try another clipboard source.');
                return;
            }
            e.preventDefault();
            await stageClipboardFiles(extraction.files);
            setAttachmentError(null);
        },
        [stageClipboardFiles],
    );
    const openStartFeature = () => {
        uiDispatch({
            type: 'OPEN_START_FEATURE',
            seed: {
                title: featureInput.trim() || undefined,
                attachments: attachments.length > 0 ? attachments : undefined,
            },
        });
        setFeatureInput('');
        setAttachments([]);
    };

    // Retry and recovery states
    const [localBootstrapStep, setLocalBootstrapStep] = useState<'idle' | 'bootstrapping' | 'strategy_proposal' | 'error'>('idle');
    const [bootstrapError, setBootstrapError] = useState('');

    // Strategy Form States
    const [defaultBranch, setDefaultBranch] = useState('');
    const [branchPrefix, setBranchPrefix] = useState('');
    const [testCommand, setTestCommand] = useState('');
    const [prTemplate, setPrTemplate] = useState('');
    const [conflictPolicy, setConflictPolicy] = useState('always_gate');
    const [featureLifecycle, setFeatureLifecycle] = useState('archive');


    const handleRetryBootstrap = async () => {
        setLocalBootstrapStep('bootstrapping');
        setBootstrapError('');
        // Preserves a value the user edited in a prior strategy_proposal
        // round before this call's fetches (which may reset the state) run.
        const currentDefaultBranch = defaultBranch;
        const currentBranchPrefix = branchPrefix;
        const currentTestCommand = testCommand;
        const currentPrTemplate = prTemplate;
        try {
            // Read existing settings so we preserve user-customized values
            const existing = await getProposedStrategy(activeProject.id);

            const strategy = await bootstrapProject(activeProject.id);

            const ext = existing?.worktree_strategy;
            setDefaultBranch(currentDefaultBranch || ext?.default_branch || strategy.default_branch);
            setBranchPrefix(currentBranchPrefix || ext?.branch_prefix || strategy.branch_prefix);
            setTestCommand(currentTestCommand || ext?.test_command || strategy.test_command || '');
            setPrTemplate(currentPrTemplate || ext?.pr_template || strategy.pr_template || '');
            setLocalBootstrapStep('strategy_proposal');
        } catch (err) {
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
            projDispatch({ type: 'UPDATE_PROJECTS', updater: prev => prev.map(p => p.id === activeProject.id ? { ...p, status: 'idle' } : p) });
            setLocalBootstrapStep('idle');
        } catch (err) {
            setLocalBootstrapStep('error');
            setBootstrapError(formatError(err));
        }
    };

    useEffect(() => {
        // Drop the outgoing project's repo list/selection before the fetch,
        // not after: this component is reused across projects (App renders it
        // without a `key`), and the header Start Session button launches at
        // `activeRepoPath`. Leaving the old value in place would let a click
        // during the fetch — or after a failed/empty fetch — open a session in
        // the project the user just left. An empty path disables the button.
        setRepositories([]);
        setRepositoriesProjectId(null);
        setActiveRepositoryId('');

        // An earlier project's request may settle after this effect has been
        // replaced. Its repositories must not become the selected launch
        // target for the newly active project.
        let cancelled = false;

        const fetchWorkspaceData = async () => {
            setIsLoadingFeatures(true);

            const [featuresRes, reposRes, workflowsRes, mirrorsRes] = await Promise.allSettled([
                fetchActiveFeatures(activeProject.id),
                getRepositoriesForProject(activeProject.id),
                listWorkflows(),
                listMirroredRuns(),
            ]);

            if (cancelled) return;

            // Detached runs: any active feature that has a remote-run mirror.
            if (mirrorsRes.status === 'fulfilled' && Array.isArray(mirrorsRes.value)) {
                setDetachedIds(
                    new Set(
                        mirrorsRes.value
                            .map((m) => m.feature_id)
                            .filter((id): id is string => !!id),
                    ),
                );
            } else {
                setDetachedIds(new Set());
            }

            // Handle active features
            if (featuresRes.status === 'fulfilled' && featuresRes.value) {
                const res = featuresRes.value;
                if (res && res.length > 0) {
                    const mapped: Feature[] = res.map((f) => ({
                        id: f.id,
                        project_id: f.project_id,
                        workflow_id: f.workflow_id ?? undefined,
                        title: f.title,
                        description: f.description ?? '',
                        status: f.status,
                        total_cost: f.total_cost,
                        tokens: f.tokens || 0,
                        duration: f.duration,
                        created_at: f.created_at,
                        agent_kind: f.agent_kind,
                        model: f.model,
                        // Carried so the card can tell a published run from a
                        // bare completed one — see `featureRunStatus`.
                        mr_url: f.mr_url ?? null,
                        mr_state: f.mr_state ?? null,
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
                setRepositories(reposRes.value);
                setRepositoriesProjectId(activeProject.id);
                if (reposRes.value.length > 0) {
                    setActiveRepositoryId(reposRes.value[0].id);
                }
            } else if (reposRes.status === 'rejected') {
                console.error("Failed to fetch repositories:", reposRes.reason);
            }

            // Handle workflows — only the id → label lookup is needed
            // here now (the launcher's workflow picker lives in the modal).
            if (workflowsRes.status === 'fulfilled' && workflowsRes.value) {
                setWorkflowById(buildWorkflowById(workflowsRes.value));
            } else if (workflowsRes.status === 'rejected') {
                console.error("Failed to fetch workflows:", workflowsRes.reason);
                setWorkflowById(new Map());
            }
        };
        fetchWorkspaceData();
        return () => {
            cancelled = true;
        };
    }, [activeProject.id]);

    // The active ID is meaningful only within the project that supplied this
    // repository list. The explicit ownership check closes the render between
    // a project change and its reset effect, when React still holds old state.
    const activeRepository = repositoriesProjectId === activeProject.id
        ? repositories.find((repo) => repo.id === activeRepositoryId) ?? null
        : null;
    const activeRepoPath = activeRepository?.repo_path ?? '';

    const isCurrentlyFailed = activeProject.status === 'error';
    const isCurrentlyBootstrapping = activeProject.status === 'bootstrapping';

    const currentStep = localBootstrapStep !== 'idle' ? localBootstrapStep :
                        isCurrentlyFailed ? 'error' :
                        isCurrentlyBootstrapping ? 'bootstrapping' : 'idle';

    // Same local-vs-remote derivation TerminalTabOpener uses below — kept as
    // one source of truth so the hero button and the auto-opened terminal
    // tab always agree on which machine a session targets.
    const machineId = activeProject.compute_type?.toLowerCase() === 'remote'
        ? (activeProject.remote_host || 'local')
        : 'local';

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
                        <button onClick={() => navigate({ kind: 'project-settings' })} className="px-5 py-2.5 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all flex items-center gap-1.5 font-medium">
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
                                onClick={() => navigate({ kind: 'project-settings' })}
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
                        <div className="flex flex-col"><span className="text-slate-500">Token Spend</span><span className="text-white">{formatTokens(features.reduce((sum, f) => sum + (f.tokens || 0), 0))}</span></div>
                    </div>
                </div>

                {/* Persistent Start Session affordance — visible for both local
                    and remote projects, regardless of which tab (if any) is
                    active. This is the only terminal entry point local projects
                    ever see; TerminalTabOpener below stays remote/'terminal'-tab
                    only and keeps its own auto-open behavior untouched. */}
                <div className="flex items-center gap-3 shrink-0">
                    {repositories.length > 1 && (
                        <div className="flex items-center gap-2 text-xs font-mono text-slate-400 bg-white/5 border border-white/5 rounded-lg p-2.5">
                            <span>Repository:</span>
                            <select
                                value={activeRepositoryId}
                                onChange={(e) => setActiveRepositoryId(e.target.value)}
                                className="bg-[#08090c] border border-white/10 rounded px-2.5 py-1 text-xs text-white focus:outline-none focus:border-cyan-500/50"
                            >
                                {repositories.map((repo) => (
                                    <option key={repo.id} value={repo.id}>
                                        {repo.repo_path}
                                    </option>
                                ))}
                            </select>
                        </div>
                    )}
                    <StartSessionButton
                        projectId={activeProject.id}
                        repositoryId={activeRepository?.id ?? ''}
                        repoPath={activeRepoPath}
                        machineId={machineId}
                        machineLabel={machineId}
                    />
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
                        {/* Inline composer — a lightweight seed for the
                            single launch surface. It captures a title +
                            attachments, then hands off to the Start
                            Feature modal, which owns every launch knob
                            and the actual start_feature call. Ctrl/Cmd+T
                            opens the same modal with no seed. */}
                        <div className="glass-panel rounded-2xl p-4 relative group overflow-hidden">
                            <div className="absolute inset-0 bg-gradient-to-r from-violet-500/10 to-cyan-500/10 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none"></div>
                            <div className="relative flex items-start gap-4">
                                <div className="mt-2 ml-1 text-violet-400 shrink-0">
                                    <Zap className="w-5 h-5" />
                                </div>
                                <div
                                    className="flex-1 min-w-0"
                                    tabIndex={0}
                                    onPaste={handleComposerPaste}
                                    data-testid="project-home-composer"
                                >
                                    <input
                                        type="text"
                                        value={featureInput}
                                        onChange={(e) => setFeatureInput(e.target.value)}
                                        onKeyDown={(e) => {
                                            if (e.key === 'Enter') {
                                                e.preventDefault();
                                                openStartFeature();
                                            }
                                        }}
                                        placeholder="Draft and delegate a new feature pipeline..."
                                        className="w-full bg-transparent border-none p-2 text-sm text-white placeholder-slate-500 focus:outline-none"
                                    />
                                    {/* Collapsed chip row: staged attachments
                                        ride along into the modal seed. Use the
                                        modal to add more once it opens. */}
                                    <div className="px-2 pb-1 flex flex-wrap items-center gap-2">
                                        <span className="text-[10px] font-mono text-slate-500 uppercase tracking-wider">Attachments</span>
                                        <AttachmentDropzone
                                            mode="launch"
                                            compact
                                            stageEntries={attachments}
                                            onChangeStage={setAttachments}
                                            maxChips={6}
                                        />
                                    </div>
                                    {attachmentError && (
                                        <p role="alert" className="px-2 text-[11px] font-mono text-ruby-200">
                                            {attachmentError}
                                        </p>
                                    )}
                                    <div className="mt-2 flex items-center justify-between gap-3 pl-2">
                                        <span className="text-[11px] text-slate-500 font-mono">
                                            Press <kbd className="px-1 py-0.5 rounded bg-white/5 border border-white/10 text-slate-400">Enter</kbd> to configure &amp; launch · paste an image to attach
                                        </span>
                                        <button
                                            onClick={openStartFeature}
                                            className="px-4 py-2 text-sm font-medium bg-violet-600 hover:bg-violet-500 text-white rounded-md shadow-[0_0_15px_rgba(139,92,246,0.4)] transition-all flex items-center gap-1.5"
                                        >
                                            Continue <ChevronRight className="w-4 h-4" />
                                        </button>
                                    </div>
                                </div>
                            </div>
                        </div>

                {/* Feature pipeline list. Not running-only — `fetch_active_features`
                    returns everything that isn't archived/deleted, so completed,
                    failed and gated runs are here too, each wearing its own chip. */}
                <div>
                    <h2 className="font-outfit text-sm font-semibold text-slate-400 uppercase tracking-widest mb-4">Feature Pipelines</h2>
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
                                <h3 className="font-outfit text-white font-medium text-base">No feature pipelines yet</h3>
                                <p className="text-xs text-slate-400 max-w-sm mx-auto leading-relaxed">
                                    There are no agent orchestration workflows running in this workspace right now. Use the tool above to start a new pipeline.
                                </p>
                            </div>
                        ) : (
                            features.map((feature) => {
                                const meta = runStatusMeta(featureRunStatus(feature));
                                return (
                                <div
                                    key={feature.id}
                                    onClick={() => {
                                        navigate({ kind: 'detail', featureId: feature.id, featureTitle: feature.title });
                                    }}
                                    className="glass-panel glass-panel-hover rounded-xl p-5 cursor-pointer relative overflow-hidden group"
                                >
                                    <div className={`absolute left-0 top-0 bottom-0 w-1 ${TONE_ACCENT[meta.tone]}`}></div>

                                    <div className="flex justify-between items-start gap-4">
                                        <div className="min-w-0 flex-1">
                                            <div className="flex items-center gap-3 mb-1 flex-wrap">
                                                <span className={`px-2 py-0.5 rounded text-[10px] font-mono border uppercase flex items-center gap-1 ${TONE_CHIP[meta.tone]}`}>
                                                    {meta.active && (
                                                        <span className="w-1.5 h-1.5 rounded-full bg-current animate-pulse"></span>
                                                    )}
                                                    {meta.label}
                                                </span>
                                                {(() => {
                                                    const badge = classifyWorkflowBadge(feature, workflowById);
                                                    if (badge.variant === 'fallback') {
                                                        return (
                                                            <span
                                                                className="px-2 py-0.5 rounded text-[10px] font-mono bg-white/5 border border-white/10 text-slate-500 uppercase"
                                                                title="Workflow reference missing"
                                                            >
                                                                Workflow: unknown
                                                            </span>
                                                        );
                                                    }
                                                    return (
                                                        <span
                                                            className="px-2 py-0.5 rounded text-[10px] font-mono bg-violet-500/10 border border-violet-500/30 text-violet-300 font-outfit truncate max-w-[220px] inline-flex items-center gap-1"
                                                            title={`Workflow: ${badge.name}`}
                                                        >
                                                            <span className="text-violet-400/80">Workflow:</span>
                                                            <span className="truncate">{badge.name}</span>
                                                            <span className="text-[9px] px-1 rounded bg-violet-500/20 text-violet-300 font-medium font-mono uppercase">
                                                                {badge.is_starter ? 'Starter' : 'Custom'}
                                                            </span>
                                                        </span>
                                                    );
                                                })()}
                                                {(() => {
                                                    const detached = detachedIds.has(feature.id);
                                                    const remote = detached || activeProject.compute_type === 'remote';
                                                    const label = detached
                                                        ? 'Detached'
                                                        : activeProject.compute_type === 'remote'
                                                        ? 'Remote · SSH'
                                                        : 'Local';
                                                    return (
                                                        <span
                                                            className={`px-2 py-0.5 rounded text-[10px] font-mono uppercase border inline-flex items-center gap-1 ${
                                                                remote
                                                                    ? 'bg-cyan-500/10 text-cyan-400 border-cyan-500/20'
                                                                    : 'bg-white/5 text-slate-500 border-white/10'
                                                            }`}
                                                            title={
                                                                detached
                                                                    ? 'Runs detached under the runner — continues even if this app is closed'
                                                                    : activeProject.compute_type === 'remote'
                                                                    ? `Executes on ${activeProject.remote_host ?? 'the project machine'} over SSH`
                                                                    : 'Executes on this machine'
                                                            }
                                                        >
                                                            <Cpu className="w-3 h-3" /> {label}
                                                        </span>
                                                    );
                                                })()}
                                                <span className="text-xs text-slate-500 font-mono truncate">{feature.id}</span>
                                            </div>
                                            <h3 className="text-lg font-outfit text-white line-clamp-2 break-words" title={feature.title}>{feature.title}</h3>
                                            {feature.description?.trim() && (
                                                <p
                                                    className="mt-1 text-xs text-slate-400 leading-relaxed line-clamp-2 break-words"
                                                    title={feature.description}
                                                >
                                                    {feature.description}
                                                </p>
                                            )}
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
                                );
                            })
                        )}
                    </div>
                </div>
                </div>
                ) : (
                    <div className="flex-1 min-h-0 flex flex-col gap-4">
                        <TerminalTabOpener
                            projectId={activeProject.id}
                            computeType={activeProject.compute_type || 'local'}
                            remoteHost={activeProject.remote_host || null}
                            repoPath={activeRepoPath}
                        />
                    </div>
                )}

            </div>

        </div>
    );
};

// Thin view that opens a terminal tab in the global panel whenever the
// Terminal tab is active (spec §3 (b)). Replaces the legacy
// `<TerminalWindow>` mount that used to live inline — the panel now owns
// session lifecycle and the xterm canvas (`TerminalSurface`).
function TerminalTabOpener({
  projectId,
  computeType,
  remoteHost,
  repoPath,
}: {
  projectId: string;
  computeType: string;
  remoteHost: string | null;
  repoPath: string;
}): React.ReactElement {
  const { open: openTerminalTab } = useTerminalPanel();
  const { navigate } = useNavigation();
  const machineId = computeType.toLowerCase() === 'remote' ? remoteHost || 'local' : 'local';

  // Open a panel tab on mount, then route to the full-page Terminals
  // view where the live surface renders. The panel owns session
  // lifecycle; this view is just a trigger that registers the intent and
  // hands off to the Terminals view.
  useEffect(() => {
    if (!repoPath) return;
    void openTerminalTab({
      machineId,
      machineLabel: machineId,
      projectId,
      repoPath,
    });
    navigate({ kind: 'terminals' });
    // The opener fires once per (project, repo, machine) tuple — the
    // panel decides whether to reuse or replace an existing tab.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId, repoPath, machineId]);

  return (
    <div className="flex-1 min-h-0 flex flex-col items-center justify-center gap-3 rounded-xl border border-white/5 bg-[#050608] p-8 text-center">
      <Terminal className="w-6 h-6 text-cyan-400" />
      <p className="text-sm font-medium text-white">Opening the Terminals view…</p>
      <p className="text-xs text-slate-500 max-w-sm">
        The PTY for <span className="font-mono text-slate-300">{repoPath || 'this repo'}</span>{' '}
        is running in the global terminal panel. Its live surface is on the
        full-page Terminals view; press{' '}
        <kbd className="px-1 py-0.5 rounded bg-white/5 border border-white/10 text-slate-400 font-mono">Cmd/Ctrl + `</kbd>{' '}
        to jump there any time, or close the active tab to end the session.
      </p>
    </div>
  );
}

export default ProjectHome;
