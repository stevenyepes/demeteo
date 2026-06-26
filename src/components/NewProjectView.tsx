import React, { useState, useEffect } from 'react';
import { Search, Plus, GitBranch, GitPullRequest, Check, X, Box, HardDrive, Server, RotateCw, AlertTriangle, Key } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { formatError } from '../lib/errors';
import { useErrorBus } from '../lib/errorBus';
import { saveProjectSettings } from '../lib/project';

interface Provider {
    id: string;
    type: string;
    name: string;
    host: string;
    pat: string;
    username: string;
    avatarUrl: string;
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
}

interface NewProjectViewProps {
    setView: (view: string) => void;
    setProjects: (updater: (prev: any[]) => any[]) => void;
    setCurrentProjectId: (id: string) => void;
    providers: Provider[];
    /**
     * Optional callback to navigate to the Machines settings page.
     * When omitted, the "Manage machines…" link is hidden — useful
     * for tests and for surfaces that don't want to expose it.
     */
    onOpenMachinesSettings?: () => void;
}

interface AvailableRepo {
    path: string;
    providerId: string;
}

interface WorktreeStrategy {
    default_branch: string;
    branch_prefix: string;
    test_command: string | null;
    pr_template: string | null;
}

const NewProjectView: React.FC<NewProjectViewProps> = ({ setView, setProjects, setCurrentProjectId, providers, onOpenMachinesSettings }) => {
    const { reportError } = useErrorBus();
    const [projectName, setProjectName] = useState('');
    const [computeType, setComputeType] = useState('local');
    const [remoteHost, setRemoteHost] = useState('');
    const [machines, setMachines] = useState<Machine[]>([]);
    const [keyPassphrase, setKeyPassphrase] = useState('');
    const [selectedRepos, setSelectedRepos] = useState<AvailableRepo[]>([]);
    const [isRepoModalOpen, setIsRepoModalOpen] = useState(false);
    const [repoSearch, setRepoSearch] = useState('');
    const [availableRepos, setAvailableRepos] = useState<AvailableRepo[]>([]);
    const [isLoadingRepos, setIsLoadingRepos] = useState(false);

    // Bootstrap Steps: 'form' | 'bootstrapping' | 'strategy_proposal' | 'error'
    const [bootstrapStep, setBootstrapStep] = useState<'form' | 'bootstrapping' | 'strategy_proposal' | 'error'>('form');
    const [bootstrapError, setBootstrapError] = useState('');
    const [projectId, setProjectId] = useState('');

    // Strategy Form States
    const [defaultBranch, setDefaultBranch] = useState('main');
    const [branchPrefix, setBranchPrefix] = useState('demeteo/features/');
    const [testCommand, setTestCommand] = useState('');
    const [prTemplate, setPrTemplate] = useState('');
    const [conflictPolicy, setConflictPolicy] = useState('always_gate');
    const [featureLifecycle, setFeatureLifecycle] = useState('archive');

    const fetchRepos = async () => {
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
            reportError(err, { kind: "internal" });
        } finally {
            setIsLoadingRepos(false);
        }
    };

    useEffect(() => {
        fetchRepos();
    }, [providers]);

    // Fetch the machine list so the Remote SSH row can show a dropdown
    // and surface a passphrase field when the selected machine uses
    // SSH-key auth (the most common case for "private key with a
    // passphrase" the user mentioned).
    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const list: Machine[] = await invoke('get_machines');
                if (!cancelled) setMachines(list ?? []);
            } catch (err) {
                reportError(err, { kind: "internal" });
            }
        })();
        return () => { cancelled = true; };
    }, []);

    // The dropdown's selected machine is what the passphrase is for.
    // Fall back to a manual id if the user typed something we don't
    // recognise (e.g. they created the machine in a previous session).
    const selectedMachine = machines.find((m) => m.id === remoteHost) ?? null;
    const showPassphraseField =
        computeType === 'remote' &&
        selectedMachine !== null &&
        selectedMachine.auth_type === 'key';

    const [isTestingConnection, setIsTestingConnection] = useState(false);
    const [connectionStatus, setConnectionStatus] = useState<'idle' | 'success' | 'error'>('idle');

    useEffect(() => {
        setConnectionStatus('idle');
    }, [remoteHost]);

    // Clear the in-memory passphrase if the user picks a different
    // machine (or switches back to local) — otherwise the next submit
    // would write the old machine's passphrase to the new one.
    useEffect(() => {
        setKeyPassphrase('');
    }, [remoteHost, computeType]);

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
        } finally {
            setIsTestingConnection(false);
        }
    };

    const handleCreate = async () => {
        if (!projectName || selectedRepos.length === 0) return;

        setBootstrapStep('bootstrapping');
        setBootstrapError('');

        try {
            // 0. If the user provided a passphrase for a key-auth
            //    machine, persist it to the keyring *before* the
            //    bootstrap clone runs (the SSH layer reads the
            //    secret during bootstrap).
            if (computeType === 'remote' && keyPassphrase.trim().length > 0 && remoteHost) {
                await invoke('set_machine_secret', {
                    machineId: remoteHost,
                    secret: keyPassphrase,
                });
                // Clear the in-memory passphrase; we don't want it
                // lingering in component state once it's been written
                // to the keyring.
                setKeyPassphrase('');
            }

            // 1. Create the project record
            const res = await invoke<{ id: string; success: boolean }>('create_project', {
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

            if (res.success) {
                setProjectId(res.id);
                // 2. Perform the clone & strategy detection
                const strategy = await invoke<WorktreeStrategy>('bootstrap_project', {
                    projectId: res.id
                });

                // 3. Set strategy proposal state
                setDefaultBranch(strategy.default_branch);
                setBranchPrefix(strategy.branch_prefix);
                setTestCommand(strategy.test_command || '');
                setPrTemplate(strategy.pr_template || '');
                setBootstrapStep('strategy_proposal');
            } else {
                throw new Error("Failed to initialize project record");
            }
        } catch (err: any) {
            setBootstrapStep('error');
            setBootstrapError(formatError(err));
        }
    };

    const handleApproveStrategy = async () => {
        try {
            // Utility merges with existing DB values, so we only pass the
            // fields shown in the strategy-proposal form. Everything else
            // (harnesses, build_command, etc.) gets a sensible default on a
            // first bootstrap or is preserved on a re-bootstrap.
            await saveProjectSettings(projectId, {
                default_branch: defaultBranch,
                branch_prefix: branchPrefix,
                test_command: testCommand || null,
                pr_template: prTemplate || null,
                conflict_policy: conflictPolicy,
                feature_lifecycle: featureLifecycle,
            });

            const newProj = {
                id: projectId,
                name: projectName,
                status: 'idle',
                repos: selectedRepos.length,
                nodes: 0,
                spend: 0.00
            };
            setProjects(prev => [...prev, newProj]);
            setCurrentProjectId(projectId);
            setView('home');
        } catch (err: any) {
            setBootstrapStep('error');
            setBootstrapError(formatError(err));
        }
    };

    if (bootstrapStep === 'bootstrapping') {
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
                            <span>Cloning Git Repositories ({selectedRepos.length} configured)...</span>
                        </div>
                        <div className="flex items-center gap-2">
                            <span className="w-2 h-2 rounded-full bg-slate-600"></span>
                            <span className="text-slate-500">Parsing PR Templates & Branch configs...</span>
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

    if (bootstrapStep === 'error') {
        return (
            <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
                <div className="glass-panel max-w-lg w-full p-8 rounded-xl flex flex-col items-center text-center relative border border-ruby-500/20 shadow-2xl">
                    <AlertTriangle className="w-12 h-12 text-ruby-400 mb-4" />
                    <h2 className="text-2xl font-outfit font-bold text-white mb-2">Bootstrap Failed</h2>
                    <p className="text-sm text-slate-400 mb-6">
                        An error occurred while building the project workspace.
                    </p>
                    <div className="w-full bg-black/40 border border-ruby-500/10 rounded-lg p-4 font-mono text-left text-xs text-ruby-300 overflow-x-auto mb-6">
                        {bootstrapError}
                    </div>
                    <div className="flex gap-3">
                        <button onClick={() => setBootstrapStep('form')} className="px-5 py-2.5 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all">
                            Back to Settings
                        </button>
                        <button onClick={handleCreate} className="px-5 py-2.5 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg transition-all font-medium">
                            Retry Build
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="flex-1 overflow-y-auto p-8 relative flex items-center justify-center">
            <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[400px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none"></div>

            {/* Repo Selection Modal */}
            {isRepoModalOpen && (
                <div className="absolute inset-0 z-50 flex items-center justify-center bg-[#08090c]/80 backdrop-blur-sm">
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

            <div className="max-w-5xl w-full grid grid-cols-1 lg:grid-cols-2 gap-8 z-10">
                {/* Left Panel: Basic Configuration */}
                <div className="space-y-6">
                    <div>
                        <h1 className="text-3xl font-outfit font-bold text-white mb-2">Project Bootstrap</h1>
                        <p className="text-sm text-slate-400">Map your repositories to a secure orchestrator workspace.</p>
                    </div>

                    <div className="glass-panel p-6 rounded-xl space-y-5">
                        <div>
                            <label className="text-xs font-mono text-slate-400 uppercase tracking-widest mb-2 block">Project Name</label>
                            <input
                                type="text"
                                value={projectName}
                                onChange={e => setProjectName(e.target.value)}
                                placeholder="e.g. billing-service-rust"
                                className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white placeholder-slate-600 focus:outline-none focus:border-violet-500/50 transition-all"
                                disabled={bootstrapStep !== 'form'}
                            />
                        </div>

                        <div>
                            <label className="text-xs font-mono text-slate-400 uppercase tracking-widest mb-2 block">Environment / Target Server</label>
                            <div className="flex gap-2">
                                <button
                                    onClick={() => setComputeType('local')}
                                    className={`flex-1 flex items-center justify-center gap-2 border rounded-lg p-3 text-sm transition-all ${computeType === 'local' ? 'bg-violet-500/10 border-violet-500/50 text-violet-300' : 'bg-black/40 border-white/5 text-slate-400'
                                        }`}
                                    disabled={bootstrapStep !== 'form'}
                                >
                                    <HardDrive className="w-4 h-4" /> Local Compute
                                </button>
                                <button
                                    onClick={() => setComputeType('remote')}
                                    className={`flex-1 flex items-center justify-center gap-2 border rounded-lg p-3 text-sm transition-all ${computeType === 'remote' ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-300' : 'bg-black/40 border-white/5 text-slate-400'
                                        }`}
                                    disabled={bootstrapStep !== 'form'}
                                >
                                    <Server className="w-4 h-4" /> Remote SSH
                                </button>
                            </div>
                            {computeType === 'remote' && (
                                <div className="mt-3 space-y-2">
                                    <div className="flex gap-2">
                                        <select
                                            value={remoteHost}
                                            onChange={e => setRemoteHost(e.target.value)}
                                            disabled={bootstrapStep !== 'form' || machines.length === 0}
                                            className="flex-1 min-w-0 bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white font-mono focus:outline-none focus:border-cyan-500/50 disabled:opacity-60"
                                        >
                                            <option value="">
                                                {machines.length === 0 ? 'No machines configured — add one in Settings → Machines' : 'Select a machine…'}
                                            </option>
                                            {machines.map(m => (
                                                <option key={m.id} value={m.id}>
                                                    {m.name} ({m.username}@{m.host}:{m.port} — {m.auth_type})
                                                </option>
                                            ))}
                                        </select>
                                        <button
                                            type="button"
                                            onClick={handleTestConnection}
                                            disabled={!remoteHost || isTestingConnection || bootstrapStep !== 'form'}
                                            title={
                                                isTestingConnection
                                                    ? 'Testing SSH connection…'
                                                    : connectionStatus === 'success'
                                                        ? 'SSH connection successful — click to re-test'
                                                        : connectionStatus === 'error'
                                                            ? 'SSH connection failed — click to retry'
                                                            : 'Test SSH connection to the selected machine'
                                            }
                                            aria-label="Test SSH connection"
                                            className="px-3 py-2 text-xs font-semibold rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-white disabled:opacity-40 flex items-center gap-1.5 transition-all shrink-0"
                                        >
                                            {isTestingConnection ? (
                                                <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                                            ) : connectionStatus === 'success' ? (
                                                <Check className="w-3.5 h-3.5 text-emerald-400" />
                                            ) : connectionStatus === 'error' ? (
                                                <X className="w-3.5 h-3.5 text-ruby-400" />
                                            ) : (
                                                <Server className="w-3.5 h-3.5 text-cyan-400" />
                                            )}
                                            <span className="hidden sm:inline">Test</span>
                                        </button>
                                    </div>

                                    {/* Connection status — small inline chip
                                        below the row so the row itself stays
                                        compact and never overlaps other UI. */}
                                    {connectionStatus !== 'idle' && (
                                        <div
                                            className={`text-[11px] font-mono flex items-center gap-1.5 ${
                                                connectionStatus === 'success'
                                                    ? 'text-emerald-400'
                                                    : 'text-ruby-400'
                                            }`}
                                        >
                                            {connectionStatus === 'success' ? (
                                                <>
                                                    <Check className="w-3 h-3" />
                                                    SSH connection verified
                                                </>
                                            ) : (
                                                <>
                                                    <X className="w-3 h-3" />
                                                    SSH connection failed — verify the machine's credentials in Settings
                                                </>
                                            )}
                                        </div>
                                    )}

                                    {onOpenMachinesSettings && (
                                        <button
                                            type="button"
                                            onClick={onOpenMachinesSettings}
                                            className="text-[11px] text-cyan-400 hover:text-cyan-300 font-mono underline-offset-2 hover:underline"
                                        >
                                            Manage machines… (open Settings)
                                        </button>
                                    )}

                                    {/* Passphrase for key-auth machines. Saved to the
                                        keyring via `set_machine_secret` immediately
                                        before the bootstrap clone runs. */}
                                    {showPassphraseField && (
                                        <div>
                                            <label className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5 flex items-center gap-1.5">
                                                <Key className="w-3 h-3" /> Private Key Passphrase
                                            </label>
                                            <input
                                                type="password"
                                                value={keyPassphrase}
                                                onChange={e => setKeyPassphrase(e.target.value)}
                                                placeholder="Leave blank if the key has no passphrase, or to keep the stored one"
                                                autoComplete="off"
                                                className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                                                disabled={bootstrapStep !== 'form'}
                                            />
                                            {selectedMachine?.key_path && (
                                                <p className="mt-1 text-[10px] text-slate-500 font-mono truncate">
                                                    Key: {selectedMachine.key_path}
                                                </p>
                                            )}
                                        </div>
                                    )}

                                    {/* If the typed id is not a known machine,
                                        surface a hint to the user. */}
                                    {computeType === 'remote' && remoteHost && !selectedMachine && machines.length > 0 && (
                                        <p className="text-[10px] text-amber-400 font-mono">
                                            Unknown machine id. Configure it in Preferences → Machines before bootstrapping.
                                        </p>
                                    )}
                                </div>
                            )}
                        </div>

                        <div>
                            <label className="text-xs font-mono text-slate-400 uppercase tracking-widest mb-2 flex justify-between items-center">
                                <span>Select Repositories</span>
                                <span className="text-cyan-500">{selectedRepos.length} Mapped</span>
                            </label>
                            <div className="space-y-2">
                                {selectedRepos.map(repo => (
                                    <div key={repo.path} className="flex items-center gap-3 p-3 rounded-lg border border-cyan-500/30 bg-cyan-500/5">
                                        <Box className="w-4 h-4 text-cyan-400" />
                                        <span className="text-sm text-white truncate w-4/5">{repo.path}</span>
                                        {bootstrapStep === 'form' && (
                                            <button onClick={() => toggleRepo(repo)} className="ml-auto text-slate-500 hover:text-ruby-400 transition-all">
                                                <X className="w-4 h-4" />
                                            </button>
                                        )}
                                    </div>
                                ))}
                                {bootstrapStep === 'form' && (
                                    <button 
                                        onClick={() => {
                                            setIsRepoModalOpen(true);
                                            if (availableRepos.length === 0) {
                                                fetchRepos();
                                            }
                                        }} 
                                        className="w-full flex items-center justify-center gap-2 p-3 rounded-lg border border-dashed border-white/10 text-slate-400 hover:text-white hover:bg-white/5 transition-all text-sm"
                                    >
                                        <Plus className="w-4 h-4" /> Manage Repositories
                                    </button>
                                )}
                            </div>
                        </div>
                    </div>
                </div>

                {/* Right Panel: Proposals & settings config. NOT sticky —
                    the left panel can grow tall (machine dropdown,
                    passphrase field, Manage-machines link, repo list)
                    and a sticky right panel would visually overlap
                    the left-panel rows as the user scrolls, blocking
                    access to the Save button. */}
                <div className="glass-panel p-6 rounded-xl flex flex-col h-fit border-white/10 shadow-2xl">
                    {bootstrapStep === 'form' ? (
                        <>
                            <div className="mb-6">
                                <h3 className="font-outfit font-semibold text-slate-400 uppercase tracking-widest text-xs mb-1">AUTOMATED PROPOSAL</h3>
                                <h2 className="text-xl font-bold text-white">Suggested Worktree Strategy</h2>
                            </div>

                            <div className="bg-black/40 rounded-lg border border-white/5 p-6 font-mono text-xs space-y-4 text-slate-300">
                                <div className="flex items-center gap-2 text-emerald-400">
                                    <GitBranch className="w-4 h-4" />
                                    <span>base: [Detected Default Branch]</span>
                                </div>
                                <div className="h-4 w-px bg-white/10 ml-2"></div>
                                <div className="flex items-center gap-2 text-violet-400">
                                    <GitPullRequest className="w-4 h-4" />
                                    <span>feature workspace: demeteo/features/*</span>
                                </div>
                                <p className="text-slate-500 leading-relaxed mt-4 pt-4 border-t border-white/5">
                                    Demeteo automatically clones and initiates branches in isolated environments. All work is restricted to agent branches. No modifications ever directly commit back to production base.
                                </p>
                            </div>

                            <div className="mt-6 flex justify-end gap-3">
                                <button onClick={() => setView('home')} className="px-5 py-2.5 text-sm font-medium text-slate-400 hover:text-white transition-colors">Cancel</button>
                                <button onClick={handleCreate} disabled={!projectName || selectedRepos.length === 0} className="disabled:opacity-40 disabled:cursor-not-allowed px-5 py-2.5 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-md shadow-[0_0_15px_rgba(16,185,129,0.3)] transition-all flex items-center gap-2">
                                    <Check className="w-4 h-4" /> Initialize & Analyze
                                </button>
                            </div>
                        </>
                    ) : (
                        <>
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
                                <button onClick={() => setBootstrapStep('form')} className="px-5 py-2.5 text-sm font-medium text-slate-400 hover:text-white transition-colors">Back</button>
                                <button onClick={handleApproveStrategy} className="px-6 py-2.5 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg shadow-[0_0_15px_rgba(16,185,129,0.3)] transition-all flex items-center gap-2">
                                    <Check className="w-4 h-4" /> Approve & Build Workspace
                                </button>
                            </div>
                        </>
                    )}
                </div>
            </div>
        </div>
    );
};

export default NewProjectView;
