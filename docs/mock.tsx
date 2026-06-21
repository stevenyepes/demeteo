import React, { useState, useEffect, useRef } from 'react';
import {
    Search, Settings, HelpCircle, Plus, LayoutGrid,
    GitBranch, GitCommit, GitMerge, Clock, Play, Pause,
    Check, X, AlertCircle, Terminal, ChevronRight, Zap,
    FileCode2, ShieldAlert, Cpu, DollarSign, Activity,
    Save, Trash2, GitPullRequest, Box, ArrowRight,
    Server, HardDrive, Globe, Key, Database, RefreshCw,
    Sliders, PlusCircle, CheckCircle2, ShieldAlert as GuardIcon
} from 'lucide-react';

const InjectStyles = () => (
    <style>{`
    @import url('https://fonts.googleapis.com/css2?family=Fira+Code:wght@400;500&family=Inter:wght@400;500;600&family=Outfit:wght@400;500;600;700&display=swap');

    :root {
      --bg-obsidian: #08090c;
      --bg-carbon: #0d0f14;
      --surface-glass: rgba(18, 22, 30, 0.75);
      --border-glow: rgba(255, 255, 255, 0.05);
      
      --accent-violet: #8b5cf6;
      --accent-cyan: #06b6d4;
      --accent-emerald: #10b981;
      --accent-ruby: #ef4444;
    }

    body {
      background-color: var(--bg-obsidian);
      color: #e2e8f0;
      font-family: 'Inter', sans-serif;
      margin: 0;
      overflow: hidden;
    }

    h1, h2, h3, h4, h5, h6, .font-outfit {
      font-family: 'Outfit', sans-serif;
    }

    code, pre, .font-mono {
      font-family: 'Fira Code', monospace;
    }

    .glass-panel {
      background: var(--surface-glass);
      backdrop-filter: blur(12px);
      -webkit-backdrop-filter: blur(12px);
      border: 1px solid var(--border-glow);
    }
    
    .glass-panel-hover:hover {
      background: rgba(25, 30, 40, 0.85);
      border-color: rgba(255,255,255,0.1);
    }

    /* Custom Scrollbar */
    ::-webkit-scrollbar { width: 6px; height: 6px; }
    ::-webkit-scrollbar-track { background: transparent; }
    ::-webkit-scrollbar-thumb { background: rgba(255,255,255,0.1); border-radius: 4px; }
    ::-webkit-scrollbar-thumb:hover { background: rgba(255,255,255,0.2); }
  `}</style>
);

const MOCK_PROJECTS = [
    { id: 'p1', name: 'core-auth-services', status: 'active', repos: 3, nodes: 4, spend: 42.50 },
    { id: 'p2', name: 'frontend-dashboard', status: 'idle', repos: 1, nodes: 0, spend: 12.00 },
    { id: 'p3', name: 'billing-pipeline', status: 'error', repos: 2, nodes: 2, spend: 104.20 },
    { id: 'p4', name: 'ml-recommendation', status: 'active', repos: 5, nodes: 8, spend: 350.00 },
];

const MOCK_PROVIDERS_INIT = [
    { id: 'prov-1', type: 'github', name: 'GitHub Enterprise Workspace', host: 'github.com', user: 'demeteo-orchestrator', status: 'connected', avatar: 'https://images.unsplash.com/photo-1618005182384-a83a8bd57fbe?w=80&auto=format&fit=crop&q=60&ixlib=rb-4.0.3' },
    { id: 'prov-2', type: 'gitlab', name: 'Self-Hosted GitLab CI', host: 'git.acme-internal.net', user: 'deploy-agent-01', status: 'connected', avatar: 'https://images.unsplash.com/photo-1579546929518-9e396f3cc809?w=80&auto=format&fit=crop&q=60&ixlib=rb-4.0.3' }
];

const MOCK_WORKFLOW_TEMPLATES = [
    {
        id: 'tpl-1',
        name: 'Standard Feature Pipeline',
        category: 'default',
        description: 'System-recommended sequence for feature implementations involving risk metrics assessment and human gates.',
        steps: [
            { id: 'ws1', type: 'agent', title: 'Research & Read Codebase', agent: 'claude-sys-1', strictness: 'High' },
            { id: 'ws2', type: 'agent', title: 'Draft Implementation Spec', agent: 'claude-sys-1', strictness: 'Medium' },
            { id: 'ws3', type: 'gate', title: 'Human Security Gate Check', required: true },
            { id: 'ws4', type: 'parallel', title: 'Scaffold Utility Stubs', agents: ['opencode-alpha', 'opencode-beta'] },
            { id: 'ws5', type: 'agent', title: 'Run Integration Tests', agent: 'hermes-worker', strictness: 'Strict' },
        ]
    },
    {
        id: 'tpl-2',
        name: 'Consensus Multi-LLM Audit',
        category: 'default',
        description: 'Runs dual code generations simultaneously and submits both diffs directly to a human Gate comparator.',
        steps: [
            { id: 'audit1', type: 'parallel', title: 'Generate Candidate Stubs', agents: ['opencode-alpha', 'claude-sys-1'] },
            { id: 'audit2', type: 'gate', title: 'A/B Diff Code Comparison Gate', required: true },
            { id: 'audit3', type: 'agent', title: 'Deploy Merged State', agent: 'hermes-worker', strictness: 'Strict' }
        ]
    },
    {
        id: 'tpl-3',
        name: 'Rapid Hotfix Bypass',
        category: 'default',
        description: 'Fully automated continuous repair cycle that completely bypasses Gate checks to restore master pipeline operations.',
        steps: [
            { id: 'hf1', type: 'agent', title: 'Identify Build failure', agent: 'claude-sys-1', strictness: 'Medium' },
            { id: 'hf2', type: 'agent', title: 'Force-Inject Source Repairs', agent: 'opencode-alpha', strictness: 'Low' },
            { id: 'hf3', type: 'agent', title: 'Run Verification Actions', agent: 'hermes-worker', strictness: 'Strict' }
        ]
    },
    {
        id: 'tpl-custom-1',
        name: 'Acme High-Performance Sandbox',
        category: 'custom',
        description: 'User-defined workflow utilizing strict high-performance local compilers and sandboxed isolation vectors.',
        steps: [
            { id: 'custom-1', type: 'agent', title: 'Local Source Decompilation', agent: 'claude-sys-1', strictness: 'High' },
            { id: 'custom-2', type: 'gate', title: 'Pre-flight Architecture Approval', required: true }
        ]
    }
];

const MOCK_FEATURES = [
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

const TopBar = ({ setView, activeTab, setActiveTab }) => (
    <header className="h-14 border-b border-white/5 bg-[#0d0f14]/80 backdrop-blur-md flex items-center justify-between px-6 z-20 relative shrink-0">
        <div className="flex items-center gap-4">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-violet-500/20 to-cyan-500/20 border border-white/10 flex items-center justify-center">
                <Activity className="w-5 h-5 text-cyan-400 animate-pulse" />
            </div>
            <h1 className="font-outfit font-bold tracking-wide text-lg text-white">demeteo</h1>
        </div>

        <div className="flex items-center gap-4">
            <div className="flex items-center px-3 py-1.5 glass-panel rounded-md text-sm text-slate-400 w-64 group hover:border-white/20 transition-colors cursor-pointer">
                <Search className="w-4 h-4 mr-2 opacity-50" />
                <span>Search workspace...</span>
                <span className="ml-auto text-[10px] font-mono border border-white/10 px-1.5 py-0.5 rounded opacity-50">⌘K</span>
            </div>
            <div className="w-px h-5 bg-white/10"></div>
            <button
                onClick={() => {
                    setView('control-panel');
                    if (setActiveTab) setActiveTab('workflows');
                }}
                className="text-slate-400 hover:text-white transition-all hover:bg-white/5 p-1.5 rounded flex items-center gap-1 text-xs"
                title="Templates Hub"
            >
                <Sliders className="w-4 h-4 text-violet-400" />
                <span className="hidden md:inline font-mono">Workflows</span>
            </button>
            <button
                onClick={() => {
                    setView('control-panel');
                    if (setActiveTab) setActiveTab('providers');
                }}
                className="text-slate-400 hover:text-white transition-all hover:bg-white/5 p-1.5 rounded flex items-center gap-1 text-xs"
                title="Source Providers"
            >
                <Globe className="w-4 h-4 text-cyan-400" />
                <span className="hidden md:inline font-mono">Providers</span>
            </button>
            <button onClick={() => setView('control-panel')} className="text-slate-400 hover:text-white transition-colors hover:bg-white/5 p-1.5 rounded">
                <Settings className="w-5 h-5" />
            </button>
            <div className="w-8 h-8 rounded-full bg-gradient-to-tr from-violet-600 to-cyan-600 border-2 border-white/10 ml-2"></div>
        </div>
    </header>
);

const Sidebar = ({ projects, currentProject, setCurrentProject, setView }) => (
    <aside className="w-64 border-r border-white/5 bg-[#0d0f14]/50 backdrop-blur-xl flex flex-col z-10 shrink-0">
        <div className="p-4 border-b border-white/5 flex justify-between items-center">
            <h2 className="text-xs font-outfit font-semibold text-slate-500 tracking-wider uppercase">Workspaces</h2>
            <div className="flex gap-1">
                <button onClick={() => setView('new-project')} className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors" title="Bootstrap Project">
                    <Plus className="w-4 h-4" />
                </button>
                <button onClick={() => setView('home')} className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors" title="Dashboard home">
                    <LayoutGrid className="w-4 h-4" />
                </button>
            </div>
        </div>
        <div className="flex-1 overflow-y-auto py-2">
            {projects.map(p => (
                <div
                    key={p.id}
                    onClick={() => {
                        setCurrentProject(p.id);
                        setView('home');
                    }}
                    className={`flex items-center justify-between px-4 py-2.5 mx-2 rounded-lg cursor-pointer transition-all duration-200 ${currentProject === p.id
                            ? 'glass-panel text-white shadow-[0_0_15px_rgba(139,92,246,0.15)]'
                            : 'text-slate-400 hover:bg-white/5 hover:text-slate-200'
                        }`}
                >
                    <div className="flex items-center gap-3">
                        <div className={`w-2 h-2 rounded-full ${p.status === 'active' ? 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]' :
                                p.status === 'error' ? 'bg-ruby-500 shadow-[0_0_8px_rgba(239,68,68,0.8)]' : 'bg-slate-600'
                            }`} />
                        <span className="text-sm font-medium truncate w-32">{p.name}</span>
                    </div>
                    <span className="text-[10px] font-mono opacity-50">{p.repos} Repos</span>
                </div>
            ))}
        </div>
    </aside>
);

const ProjectHome = ({ setView, activeProject, setActiveFeatureId }) => {
    const [featureInput, setFeatureInput] = useState('');
    const [isExpanded, setIsExpanded] = useState(false);
    const [targetRepo, setTargetRepo] = useState('All Connected');

    const activeFeatures = MOCK_FEATURES;

    return (
        <div className="flex-1 overflow-y-auto p-8 relative">
            <div className="max-w-4xl mx-auto space-y-8">

                {/* Header Block with Telemetry */}
                <div className="flex justify-between items-end">
                    <div>
                        <h1 className="text-3xl font-outfit font-bold text-white mb-2 tracking-tight">{activeProject.name}</h1>
                        <p className="text-sm text-slate-400">Connected via GitHub Enterprise &bull; Default Workflow: Standard Feature Pipeline</p>
                    </div>
                    <div className="glass-panel px-4 py-2 rounded-lg flex gap-4 text-xs font-mono">
                        <div className="flex flex-col"><span className="text-slate-500">Fleet Active</span><span className="text-emerald-400 font-bold">{activeProject.nodes} Nodes</span></div>
                        <div className="w-px bg-white/10"></div>
                        <div className="flex flex-col"><span className="text-slate-500">Cumulative Spend</span><span className="text-white">${activeProject.spend.toFixed(2)}</span></div>
                    </div>
                </div>

                {/* Start Feature Expanded Card */}
                <div className={`glass-panel rounded-2xl transition-all duration-300 ${isExpanded ? 'p-6' : 'p-2 relative group overflow-hidden'}`}>
                    {!isExpanded && (
                        <div className="absolute inset-0 bg-gradient-to-r from-violet-500/10 to-cyan-500/10 opacity-0 group-hover:opacity-100 transition-opacity"></div>
                    )}

                    <div className="relative flex items-start gap-4">
                        <div className={`mt-2 ml-2 rounded-full flex items-center justify-center transition-colors ${isExpanded ? 'bg-violet-500/20 text-violet-400 p-2' : 'text-slate-500'}`}>
                            <Zap className="w-5 h-5" />
                        </div>
                        <div className="flex-1">
                            {isExpanded ? (
                                <>
                                    <h3 className="font-outfit text-white font-medium mb-4">Start a new Feature Pipeline</h3>
                                    <textarea
                                        autoFocus
                                        value={featureInput}
                                        onChange={(e) => setFeatureInput(e.target.value)}
                                        placeholder="Describe the feature or bug you want the agent fleet to build... (e.g., 'Configure OAuth2 with Google authentication')"
                                        className="w-full bg-black/20 border border-white/5 rounded-lg p-4 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-violet-500/50 focus:ring-1 focus:ring-violet-500/50 min-h-[120px] transition-all resize-none"
                                    />

                                    {/* Auto-Inference Simulation (Journey 5) */}
                                    {featureInput.length > 5 && (
                                        <div className="mt-4 p-3 border border-cyan-500/20 bg-cyan-500/5 rounded-lg flex items-start gap-3">
                                            <Cpu className="w-4 h-4 text-cyan-400 mt-0.5 animate-pulse" />
                                            <div className="text-xs">
                                                <span className="text-slate-300">Auto-Detected Scope: </span>
                                                <span className="text-cyan-400 font-mono px-1.5 py-0.5 border border-cyan-400/30 rounded bg-cyan-400/10">auth_middleware.ts</span>
                                                <span className="text-slate-300 mx-2">| Workflow Map: </span>
                                                <span className="text-white font-medium">Standard Feature Pipeline</span>
                                            </div>
                                        </div>
                                    )}

                                    <div className="mt-4 flex justify-between items-center">
                                        <div className="flex gap-4">
                                            <select
                                                value={targetRepo}
                                                onChange={e => setTargetRepo(e.target.value)}
                                                className="bg-black/40 border border-white/10 rounded px-2 py-1 text-xs text-slate-300 focus:outline-none"
                                            >
                                                <option value="All Connected">All Connected Repos</option>
                                                <option value="api-gateway">acme-corp/api-gateway</option>
                                                <option value="auth-service">acme-corp/auth-service</option>
                                            </select>
                                        </div>
                                        <div className="flex gap-3">
                                            <button onClick={() => setIsExpanded(false)} className="px-4 py-2 text-sm font-medium text-slate-400 hover:text-white transition-colors">Cancel</button>
                                            <button
                                                onClick={() => {
                                                    setActiveFeatureId(MOCK_FEATURES[0].id);
                                                    setView('detail');
                                                }}
                                                className="px-6 py-2 text-sm font-medium bg-violet-600 hover:bg-violet-500 text-white rounded-md shadow-[0_0_15px_rgba(139,92,246,0.4)] transition-all flex items-center gap-2"
                                            >
                                                <Play className="w-4 h-4" /> Delegate Workspace
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

                {/* Active Features Tracking List */}
                <div>
                    <h2 className="font-outfit text-sm font-semibold text-slate-400 uppercase tracking-widest mb-4">Active Running Pipelines</h2>
                    <div className="space-y-4">
                        {activeFeatures.map(feature => (
                            <div
                                key={feature.id}
                                onClick={() => {
                                    setActiveFeatureId(feature.id);
                                    setView('detail');
                                }}
                                className="glass-panel glass-panel-hover rounded-xl p-5 cursor-pointer relative overflow-hidden group"
                            >
                                <div className={`absolute left-0 top-0 bottom-0 w-1 ${feature.status === 'gated' ? 'bg-violet-500 shadow-[0_0_10px_rgba(139,92,246,0.8)]' : 'bg-cyan-500 shadow-[0_0_10px_rgba(6,182,212,0.8)]'
                                    }`}></div>

                                <div className="flex justify-between items-center">
                                    <div>
                                        <div className="flex items-center gap-3 mb-1">
                                            {feature.status === 'gated' ? (
                                                <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-violet-500/10 border border-violet-500/20 text-violet-400 uppercase">GATED APPROVAL</span>
                                            ) : (
                                                <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-cyan-500/10 border border-cyan-500/20 text-cyan-400 uppercase flex items-center gap-1">
                                                    <span className="w-1.5 h-1.5 rounded-full bg-cyan-400 animate-pulse"></span> RUNNING FLEET
                                                </span>
                                            )}
                                            <span className="text-xs text-slate-500 font-mono">{feature.id}</span>
                                        </div>
                                        <h3 className="text-lg font-outfit text-white">{feature.title}</h3>
                                    </div>

                                    <div className="flex gap-6 text-right">
                                        <div>
                                            <div className="text-xs text-slate-500 font-mono flex items-center gap-1 justify-end"><Clock className="w-3 h-3" /> Duration</div>
                                            <div className="text-sm font-medium text-white">{feature.duration}</div>
                                        </div>
                                        <div>
                                            <div className="text-xs text-slate-500 font-mono flex items-center gap-1 justify-end"><DollarSign className="w-3 h-3" /> Cost</div>
                                            <div className="text-sm font-medium text-white">${feature.totalCost.toFixed(2)}</div>
                                        </div>
                                        <ChevronRight className="w-5 h-5 text-slate-500 mt-2 opacity-0 group-hover:opacity-100 transition-opacity" />
                                    </div>
                                </div>
                            </div>
                        ))}
                    </div>
                </div>

            </div>
        </div>
    );
};

const FeatureDetail = ({ setView, featureId }) => {
    const feature = MOCK_FEATURES.find(f => f.id === featureId) || MOCK_FEATURES[0];

    return (
        <div className="flex-1 overflow-y-auto flex flex-col relative">
            <div className="absolute top-[-20%] right-[-10%] w-[600px] h-[600px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none"></div>

            <div className="p-8 border-b border-white/5 glass-panel z-10 sticky top-0">
                <div className="max-w-5xl mx-auto flex items-start justify-between">
                    <div>
                        <button onClick={() => setView('home')} className="text-xs text-slate-400 hover:text-white mb-4 flex items-center gap-1 transition-colors">
                            <ChevronRight className="w-4 h-4 rotate-180" /> Return to Workspace Dashboard
                        </button>
                        <div className="flex items-center gap-3 mb-2">
                            {feature.status === 'gated' && (
                                <div className="px-2 py-0.5 rounded text-[10px] font-mono bg-violet-500/10 border border-violet-500/20 text-violet-400 flex items-center gap-1">
                                    <span className="w-1.5 h-1.5 rounded-full bg-violet-500 animate-pulse"></span> GATED WORKSPACE
                                </div>
                            )}
                            {feature.status === 'running' && (
                                <div className="px-2 py-0.5 rounded text-[10px] font-mono bg-cyan-500/10 border border-cyan-500/20 text-cyan-400 flex items-center gap-1">
                                    <span className="w-1.5 h-1.5 rounded-full bg-cyan-400 animate-pulse"></span> IN PROGRESS
                                </div>
                            )}
                            <span className="text-xs text-slate-500 font-mono tracking-widest">{feature.id}</span>
                        </div>
                        <h1 className="text-3xl font-outfit font-bold text-white tracking-tight">{feature.title}</h1>
                    </div>

                    <div className="flex gap-4">
                        <div className="glass-panel px-4 py-2 rounded-lg text-right">
                            <div className="text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1">Accrued Cost</div>
                            <div className="text-xl font-outfit text-white">${feature.totalCost.toFixed(2)}</div>
                        </div>
                        <div className="glass-panel px-4 py-2 rounded-lg text-right">
                            <div className="text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1">Duration</div>
                            <div className="text-xl font-outfit text-white">{feature.duration}</div>
                        </div>
                    </div>
                </div>
            </div>

            <div className="flex-1 p-8 max-w-5xl mx-auto w-full relative z-10">
                <h2 className="font-outfit text-sm font-semibold text-slate-400 uppercase tracking-widest mb-8 flex items-center gap-2">
                    <GitBranch className="w-4 h-4" /> Orchestration DAG Execution Graph
                </h2>

                <div className="space-y-6 relative before:absolute before:inset-0 before:ml-[1.4rem] before:h-full before:w-[2px] before:bg-white/5">
                    {feature.steps.map((step, index) => (
                        <div key={step.id} className="relative flex items-start group">

                            <div className={`flex items-center justify-center w-11 h-11 rounded-full border-4 border-[#08090c] shrink-0 z-10 shadow-[0_0_0_1px_rgba(255,255,255,0.05)] ${step.type === 'gate' ? 'bg-violet-600 shadow-[0_0_15px_rgba(139,92,246,0.5)] cursor-pointer hover:scale-105 transition-transform' :
                                    step.status === 'done' ? 'bg-[#10b981]' :
                                        step.status === 'running' ? 'bg-[#06b6d4] shadow-[0_0_15px_rgba(6,182,212,0.4)] animate-pulse' :
                                            'bg-[#1e2330]'
                                }`} onClick={step.type === 'gate' ? () => setView('gate') : undefined}>
                                {step.type === 'gate' ? <ShieldAlert className="w-5 h-5 text-white" /> :
                                    step.type === 'parallel' ? <GitMerge className="w-5 h-5 text-white" /> :
                                        step.status === 'done' ? <Check className="w-5 h-5 text-black" /> :
                                            <Terminal className="w-5 h-5 text-slate-500" />}
                            </div>

                            <div className="ml-6 w-full">
                                <div className={`glass-panel rounded-xl p-4 transition-all ${step.type === 'gate' ? 'border-violet-500/50 bg-violet-500/5 hover:bg-violet-500/10 cursor-pointer shadow-[0_0_20px_rgba(139,92,246,0.1)]' :
                                        step.status === 'running' ? 'border-cyan-500/30 bg-cyan-500/5' :
                                            'glass-panel-hover'
                                    }`} onClick={step.type === 'gate' ? () => setView('gate') : undefined}>
                                    <div className="flex justify-between items-start mb-2">
                                        <div className="flex items-center gap-2">
                                            <span className="text-[10px] font-mono tracking-widest uppercase text-slate-500">Step {index + 1}</span>
                                            {step.agent && (
                                                <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-black/40 border border-white/5 text-cyan-400">
                                                    {step.agent}
                                                </span>
                                            )}
                                            {step.type === 'gate' && (
                                                <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-violet-500/20 border border-violet-500/30 text-violet-300 flex items-center gap-1 animate-pulse">
                                                    ACTION REQUIRED
                                                </span>
                                            )}
                                        </div>

                                        <div className="flex gap-4 text-xs font-mono text-slate-400">
                                            <span className="flex items-center gap-1"><Clock className="w-3 h-3" /> {step.time}</span>
                                            <span className="flex items-center gap-1"><DollarSign className="w-3 h-3" /> {step.cost.toFixed(2)}</span>
                                        </div>
                                    </div>

                                    <h3 className={`text-base font-medium ${step.type === 'gate' ? 'text-violet-200 font-outfit text-lg' : 'text-white'
                                        }`}>
                                        {step.title}
                                    </h3>

                                    {step.type === 'parallel' && step.subtasks && (
                                        <div className="mt-4 space-y-2 pl-4 border-l-2 border-white/10">
                                            {step.subtasks.map((sub, i) => (
                                                <div key={i} className="flex items-center justify-between bg-black/20 border border-white/5 rounded p-2 text-sm">
                                                    <div className="flex items-center gap-2">
                                                        <FileCode2 className="w-4 h-4 text-slate-400" />
                                                        <span className="text-slate-200 font-mono">{sub.title}</span>
                                                    </div>
                                                    <div className="flex items-center gap-3">
                                                        <span className="text-[10px] font-mono text-cyan-500 bg-cyan-500/10 px-1.5 py-0.5 rounded">{sub.agent}</span>
                                                        {sub.status === 'done' && <Check className="w-4 h-4 text-emerald-500" />}
                                                    </div>
                                                </div>
                                            ))}
                                        </div>
                                    )}

                                    {step.type === 'gate' && (
                                        <p className="text-sm text-slate-400 mt-2">
                                            Pipeline halted. Demeteo requires verification of proposed session-auth diffs before applying commits.
                                        </p>
                                    )}
                                </div>
                            </div>
                        </div>
                    ))}
                </div>
            </div>
        </div>
    );
};

const GateView = ({ setView }) => {
    return (
        <div className="absolute inset-0 bg-[#08090c] z-50 flex flex-col animate-in slide-in-from-bottom-4 duration-300">
            <header className="h-14 border-b border-white/5 bg-[#0d0f14] flex items-center justify-between px-6 shrink-0">
                <div className="flex items-center gap-3">
                    <div className="w-8 h-8 rounded bg-violet-500/20 border border-violet-500/50 flex items-center justify-center">
                        <ShieldAlert className="w-5 h-5 text-violet-400" />
                    </div>
                    <div>
                        <h2 className="font-outfit font-bold text-white text-sm">Gate Interception Terminal</h2>
                        <p className="text-[10px] font-mono text-slate-400">Review Code & Security Assertions</p>
                    </div>
                </div>
                <button onClick={() => setView('detail')} className="p-2 text-slate-400 hover:text-white rounded-md hover:bg-white/5 transition-colors">
                    <X className="w-5 h-5" />
                </button>
            </header>

            <div className="flex-1 flex overflow-hidden">
                <div className="w-[450px] border-r border-white/5 bg-[#0d0f14]/50 p-6 flex flex-col relative">
                    <div className="absolute inset-0 bg-violet-600/5 pointer-events-none"></div>

                    <div className="flex-1 overflow-y-auto relative z-10 space-y-6">
                        <div>
                            <h3 className="font-outfit font-semibold text-slate-300 mb-2 uppercase tracking-widest text-xs">Orchestrator Synthesis</h3>
                            <div className="glass-panel rounded-xl p-4 text-sm text-slate-300 leading-relaxed border-l-2 border-l-cyan-400">
                                <p className="mb-2">
                                    <strong className="text-white">claude-sys-1</strong> completed JWT utility structures. It recommends upgrading session logic to read headers.
                                </p>
                                <p>
                                    <strong className="text-violet-400">Warning:</strong> Downstream API routes might break if legacy cookies are not supported.
                                </p>
                            </div>
                        </div>

                        <div>
                            <h3 className="font-outfit font-semibold text-slate-300 mb-2 uppercase tracking-widest text-xs">Intervention Option</h3>
                            <div className="space-y-3">
                                <label className="flex items-start gap-3 p-3 rounded-lg border border-white/10 bg-white/5 cursor-pointer hover:bg-white/10 transition-colors">
                                    <input type="radio" name="action" defaultChecked className="mt-1 accent-violet-500" />
                                    <div>
                                        <div className="text-sm font-medium text-white">Approve & Push Changes</div>
                                        <div className="text-xs text-slate-400">Accept diffs and trigger test suite execution.</div>
                                    </div>
                                </label>
                                <label className="flex items-start gap-3 p-3 rounded-lg border border-white/10 bg-white/5 cursor-pointer hover:bg-white/10 transition-colors">
                                    <input type="radio" name="action" className="mt-1 accent-cyan-500" />
                                    <div>
                                        <div className="text-sm font-medium text-white">Redirect Strategy (Instruct Agent)</div>
                                        <div className="text-xs text-slate-400">Provide direct feedback and prompt agent execution loop.</div>
                                    </div>
                                </label>
                            </div>
                        </div>

                        <div className="pt-2">
                            <textarea
                                placeholder="e.g. Include backwards compatibility with old cookies in the fallback path..."
                                className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50 min-h-[100px] resize-none"
                            />
                        </div>
                    </div>

                    <div className="pt-4 border-t border-white/10 flex gap-3 z-10">
                        <button onClick={() => setView('detail')} className="px-4 py-2 text-sm font-medium text-slate-400 hover:text-white transition-colors bg-white/5 rounded-md flex-1">Abort Run</button>
                        <button onClick={() => setView('detail')} className="px-4 py-2 text-sm font-medium bg-violet-600 hover:bg-violet-500 text-white rounded-md shadow-[0_0_15px_rgba(139,92,246,0.4)] transition-all flex-1">
                            Resume Pipeline
                        </button>
                    </div>
                </div>

                {/* Unified Code Diff Viewer */}
                <div className="flex-1 bg-[#08090c] flex flex-col">
                    <div className="h-10 border-b border-white/5 flex items-center px-4 bg-[#0d0f14]">
                        <div className="flex items-center gap-2 text-xs font-mono text-slate-400">
                            <FileCode2 className="w-4 h-4" />
                            <span>src/middleware/auth_middleware.ts</span>
                            <span className="ml-2 px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[10px]">PROPOSED DIFF</span>
                        </div>
                    </div>
                    <div className="flex-1 p-4 font-mono text-xs leading-relaxed overflow-y-auto">
                        <div className="text-slate-500">@@ -15,8 +15,12 @@</div>
                        <div className="text-slate-300 px-2 py-0.5 hover:bg-white/5">export const requireAuth = (req: Request, res: Response, next: NextFunction) ={'>'} {'{'}</div>
                        <div className="text-slate-300 px-2 py-0.5 hover:bg-white/5">  try {'{'}</div>
                        <div className="text-ruby-400 bg-ruby-500/10 px-2 py-0.5 border-l-2 border-ruby-500">-   const token = req.cookies?.session;</div>
                        <div className="text-emerald-400 bg-emerald-500/10 px-2 py-0.5 border-l-2 border-emerald-500">+   const authHeader = req.headers.authorization;</div>
                        <div className="text-emerald-400 bg-emerald-500/10 px-2 py-0.5 border-l-2 border-emerald-500">+   if (!authHeader?.startsWith('Bearer ')) {'{'}</div>
                        <div className="text-emerald-400 bg-emerald-500/10 px-2 py-0.5 border-l-2 border-emerald-500">+     return res.status(401).json({'{'} error: 'Missing token' {'}'});</div>
                        <div className="text-emerald-400 bg-emerald-500/10 px-2 py-0.5 border-l-2 border-emerald-500">+   {'}'}</div>
                        <div className="text-slate-300 px-2 py-0.5 hover:bg-white/5">    const token = authHeader.split(' ')[1];</div>
                    </div>
                </div>
            </div>
        </div>
    );
};

const NewProjectView = ({ setView, setProjects, setCurrentProjectId }) => {
    const [projectName, setProjectName] = useState('');
    const [computeType, setComputeType] = useState('local');
    const [remoteHost, setRemoteHost] = useState('');
    const [selectedRepos, setSelectedRepos] = useState(['acme-corp/api-gateway']);
    const [isRepoModalOpen, setIsRepoModalOpen] = useState(false);
    const [repoSearch, setRepoSearch] = useState('');

    const availableRepos = [
        'acme-corp/api-gateway', 'acme-corp/frontend-ui', 'acme-corp/auth-service',
        'acme-corp/billing-pipeline', 'acme-corp/infra-as-code'
    ];

    const toggleRepo = (repo) => {
        setSelectedRepos(prev =>
            prev.includes(repo) ? prev.filter(r => r !== repo) : [...prev, repo]
        );
    };

    const handleCreate = () => {
        if (!projectName) return;
        const newProj = {
            id: `p${Math.floor(Math.random() * 1000)}`,
            name: projectName,
            status: 'idle',
            repos: selectedRepos.length,
            nodes: computeType === 'local' ? 4 : 8,
            spend: 0.00
        };
        setProjects(prev => [...prev, newProj]);
        setCurrentProjectId(newProj.id);
        setView('home');
    };

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
                                    placeholder="Search providers..."
                                    className="w-full bg-black/40 border border-white/10 rounded-lg py-2.5 pl-9 pr-4 text-sm text-white focus:outline-none focus:border-cyan-500/50"
                                />
                            </div>
                        </div>
                        <div className="overflow-y-auto max-h-[300px] p-2 space-y-1 bg-[#08090c]">
                            {availableRepos.filter(r => r.includes(repoSearch.toLowerCase())).map(repo => {
                                const isSelected = selectedRepos.includes(repo);
                                return (
                                    <div
                                        key={repo}
                                        onClick={() => toggleRepo(repo)}
                                        className={`flex items-center gap-3 p-3 rounded-lg cursor-pointer transition-all ${isSelected ? 'bg-cyan-500/10 border border-cyan-500/30' : 'hover:bg-white/5 border border-transparent'
                                            }`}
                                    >
                                        <div className={`w-4 h-4 rounded border flex items-center justify-center ${isSelected ? 'bg-cyan-500 border-cyan-500 text-black' : 'border-slate-600'
                                            }`}>
                                            {isSelected && <Check className="w-3 h-3 stroke-[3]" />}
                                        </div>
                                        <Box className={`w-4 h-4 ${isSelected ? 'text-cyan-400' : 'text-slate-500'}`} />
                                        <span className={isSelected ? 'text-white' : 'text-slate-300'}>{repo}</span>
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

            <div className="max-w-4xl w-full grid grid-cols-2 gap-8 z-10">
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
                            />
                        </div>

                        <div>
                            <label className="text-xs font-mono text-slate-400 uppercase tracking-widest mb-2 block">Environment / Target Server</label>
                            <div className="flex gap-2">
                                <button
                                    onClick={() => setComputeType('local')}
                                    className={`flex-1 flex items-center justify-center gap-2 border rounded-lg p-3 text-sm transition-all ${computeType === 'local' ? 'bg-violet-500/10 border-violet-500/50 text-violet-300' : 'bg-black/40 border-white/5 text-slate-400'
                                        }`}
                                >
                                    <HardDrive className="w-4 h-4" /> Local Docker
                                </button>
                                <button
                                    onClick={() => setComputeType('remote')}
                                    className={`flex-1 flex items-center justify-center gap-2 border rounded-lg p-3 text-sm transition-all ${computeType === 'remote' ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-300' : 'bg-black/40 border-white/5 text-slate-400'
                                        }`}
                                >
                                    <Server className="w-4 h-4" /> Remote SSH
                                </button>
                            </div>
                            {computeType === 'remote' && (
                                <div className="mt-3">
                                    <input
                                        type="text"
                                        value={remoteHost}
                                        onChange={e => setRemoteHost(e.target.value)}
                                        placeholder="e.g. developer@10.27.40.51"
                                        className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white font-mono focus:outline-none"
                                    />
                                </div>
                            )}
                        </div>

                        <div>
                            <label className="text-xs font-mono text-slate-400 uppercase tracking-widest mb-2 flex justify-between items-center">
                                <span>Select Repositories</span>
                                <span className="text-cyan-500">{selectedRepos.length} Map</span>
                            </label>
                            <div className="space-y-2">
                                {selectedRepos.map(repo => (
                                    <div key={repo} className="flex items-center gap-3 p-3 rounded-lg border border-cyan-500/30 bg-cyan-500/5">
                                        <Box className="w-4 h-4 text-cyan-400" />
                                        <span className="text-sm text-white">{repo}</span>
                                        <button onClick={() => toggleRepo(repo)} className="ml-auto text-slate-500 hover:text-ruby-400 transition-all">
                                            <X className="w-4 h-4" />
                                        </button>
                                    </div>
                                ))}
                                <button onClick={() => setIsRepoModalOpen(true)} className="w-full flex items-center justify-center gap-2 p-3 rounded-lg border border-dashed border-white/10 text-slate-400 hover:text-white hover:bg-white/5 transition-all text-sm">
                                    <Plus className="w-4 h-4" /> Manage Repositories
                                </button>
                            </div>
                        </div>
                    </div>
                </div>

                {/* Strategy Proposal (Journey 3) */}
                <div className="glass-panel p-6 rounded-xl flex flex-col h-fit sticky top-8">
                    <div className="mb-6">
                        <h3 className="font-outfit font-semibold text-slate-300 uppercase tracking-widest text-xs mb-1">AUTOMATED PROPOSAL</h3>
                        <h2 className="text-lg text-white">Suggested Worktree Strategy</h2>
                    </div>

                    <div className="bg-black/40 rounded-lg border border-white/5 p-6 font-mono text-xs space-y-4">
                        <div className="flex items-center gap-2 text-emerald-400">
                            <GitBranch className="w-4 h-4" />
                            <span>base: main (upstream)</span>
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
                        <button onClick={() => setView('home')} className="px-6 py-3 text-sm font-medium text-slate-400 hover:text-white transition-colors">Cancel</button>
                        <button onClick={handleCreate} className="px-6 py-3 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-md shadow-[0_0_15px_rgba(16,185,129,0.3)] transition-all flex items-center gap-2">
                            <Check className="w-4 h-4" /> Build Workspace
                        </button>
                    </div>
                </div>
            </div>
        </div>
    );
};

const ControlPanel = ({ setView, activeTab, setActiveTab }) => {
    const [providers, setProviders] = useState(MOCK_PROVIDERS_INIT);
    const [templates, setTemplates] = useState(MOCK_WORKFLOW_TEMPLATES);
    const [selectedTemplate, setSelectedTemplate] = useState(MOCK_WORKFLOW_TEMPLATES[0]);

    // Provider Form State
    const [provType, setProvType] = useState('github');
    const [provName, setProvName] = useState('');
    const [provHost, setProvHost] = useState('github.com');
    const [provPat, setProvPat] = useState('');
    const [isTestingConn, setIsTestingConn] = useState(false);
    const [testSuccess, setTestSuccess] = useState(false);

    // Workflow Editor Form State
    const [editedTemplateName, setEditedTemplateName] = useState(selectedTemplate.name);
    const [editedDescription, setEditedDescription] = useState(selectedTemplate.description);
    const [editedSteps, setEditedSteps] = useState([...selectedTemplate.steps]);

    useEffect(() => {
        setEditedTemplateName(selectedTemplate.name);
        setEditedDescription(selectedTemplate.description);
        setEditedSteps([...selectedTemplate.steps]);
    }, [selectedTemplate]);

    // Test and Save Provider Simulation
    const handleTestAndConnect = () => {
        if (!provName || !provPat) return;
        setIsTestingConn(true);
        setTimeout(() => {
            setIsTestingConn(false);
            setTestSuccess(true);
            setTimeout(() => {
                const newProvider = {
                    id: `prov-${Math.floor(Math.random() * 1000)}`,
                    type: provType,
                    name: provName,
                    host: provHost,
                    user: provType === 'github' ? 'octocat-dev' : 'gitlab-runner-custom',
                    status: 'connected',
                    avatar: 'https://images.unsplash.com/photo-1535713875002-d1d0cf377fde?w=80&auto=format&fit=crop&q=60&ixlib=rb-4.0.3'
                };
                setProviders([...providers, newProvider]);
                setProvName('');
                setProvPat('');
                setTestSuccess(false);
            }, 1500);
        }, 2000);
    };

    // Add Step inside Workflow Editor
    const addWorkflowStep = () => {
        const newStep = {
            id: `ws-${Math.floor(Math.random() * 1000)}`,
            type: 'agent',
            title: 'New Analytical Action',
            agent: 'claude-sys-1',
            strictness: 'Medium'
        };
        setEditedSteps([...editedSteps, newStep]);
    };

    // Delete Step inside Workflow Editor
    const deleteWorkflowStep = (stepId) => {
        setEditedSteps(editedSteps.filter(s => s.id !== stepId));
    };

    // Duplicate a Template to custom selection
    const handleDuplicateTemplate = (tpl) => {
        const duplicated = {
            ...tpl,
            id: `tpl-custom-${Math.floor(Math.random() * 1000)}`,
            name: `Copy of ${tpl.name}`,
            category: 'custom'
        };
        setTemplates([...templates, duplicated]);
        setSelectedTemplate(duplicated);
    };

    // Save changes to current Custom Template
    const handleSaveWorkflow = () => {
        const updated = templates.map(t => {
            if (t.id === selectedTemplate.id) {
                return {
                    ...t,
                    name: editedTemplateName,
                    description: editedDescription,
                    steps: editedSteps
                };
            }
            return t;
        });
        setTemplates(updated);
        setSelectedTemplate({
            ...selectedTemplate,
            name: editedTemplateName,
            description: editedDescription,
            steps: editedSteps
        });
    };

    return (
        <div className="flex-1 flex flex-col overflow-hidden h-full">
            {/* Settings Navigation SubHeader */}
            <div className="p-6 border-b border-white/5 glass-panel z-10 flex justify-between items-center shrink-0">
                <div>
                    <div className="flex items-center gap-2 text-xs font-mono text-slate-400 mb-1">
                        <Sliders className="w-3 h-3 text-violet-400" /> SYSTEM CONTROL PANEL
                    </div>
                    <h1 className="text-2xl font-outfit font-bold text-white">System Settings & Workflows</h1>
                </div>
                <div className="flex gap-2">
                    <button
                        onClick={() => setActiveTab('workflows')}
                        className={`px-4 py-2 text-sm font-medium rounded-md transition-all ${activeTab === 'workflows' ? 'bg-violet-600 text-white' : 'text-slate-400 hover:text-white hover:bg-white/5'
                            }`}
                    >
                        Workflow Blueprints
                    </button>
                    <button
                        onClick={() => setActiveTab('providers')}
                        className={`px-4 py-2 text-sm font-medium rounded-md transition-all ${activeTab === 'providers' ? 'bg-violet-600 text-white' : 'text-slate-400 hover:text-white hover:bg-white/5'
                            }`}
                    >
                        Source Providers
                    </button>
                    <div className="w-px bg-white/10 mx-2"></div>
                    <button onClick={() => setView('home')} className="px-4 py-2 text-sm text-slate-400 hover:text-white transition-colors">Exit</button>
                </div>
            </div>

            <div className="flex-1 flex overflow-hidden">
                {activeTab === 'workflows' ? (
                    <div className="flex-1 flex overflow-hidden">
                        {/* Left: Templates Directory */}
                        <div className="w-80 border-r border-white/5 bg-[#0d0f14]/30 overflow-y-auto p-4 flex flex-col shrink-0">
                            <div className="mb-4">
                                <span className="text-[10px] font-mono uppercase tracking-widest text-slate-500 font-bold block mb-2">System Defaults</span>
                                <div className="space-y-1.5">
                                    {templates.filter(t => t.category === 'default').map(tpl => (
                                        <div
                                            key={tpl.id}
                                            onClick={() => setSelectedTemplate(tpl)}
                                            className={`p-3 rounded-lg border text-left cursor-pointer transition-all ${selectedTemplate.id === tpl.id
                                                    ? 'bg-violet-500/10 border-violet-500/40 text-white'
                                                    : 'bg-black/20 border-white/5 text-slate-400 hover:text-slate-200 hover:border-white/10'
                                                }`}
                                        >
                                            <h4 className="text-sm font-medium text-slate-100">{tpl.name}</h4>
                                            <p className="text-[10px] opacity-75 truncate mt-1">{tpl.description}</p>
                                        </div>
                                    ))}
                                </div>
                            </div>

                            <div className="mt-4">
                                <div className="flex justify-between items-center mb-2">
                                    <span className="text-[10px] font-mono uppercase tracking-widest text-slate-500 font-bold">User Custom Templates</span>
                                    <button
                                        onClick={() => {
                                            const newTpl = {
                                                id: `tpl-custom-${Math.floor(Math.random() * 1000)}`,
                                                name: 'My Custom Pipeline Blueprint',
                                                category: 'custom',
                                                description: 'Editable custom workflow designed for internal feature orchestration.',
                                                steps: [
                                                    { id: 'cs1', type: 'agent', title: 'Compile Local Sandbox', agent: 'opencode-alpha', strictness: 'Medium' }
                                                ]
                                            };
                                            setTemplates([...templates, newTpl]);
                                            setSelectedTemplate(newTpl);
                                        }}
                                        className="p-1 hover:bg-white/5 rounded text-violet-400 hover:text-white transition-all"
                                        title="Create custom template"
                                    >
                                        <PlusCircle className="w-4 h-4" />
                                    </button>
                                </div>
                                <div className="space-y-1.5">
                                    {templates.filter(t => t.category === 'custom').map(tpl => (
                                        <div
                                            key={tpl.id}
                                            onClick={() => setSelectedTemplate(tpl)}
                                            className={`p-3 rounded-lg border text-left cursor-pointer transition-all relative group ${selectedTemplate.id === tpl.id
                                                    ? 'bg-violet-500/10 border-violet-500/40 text-white'
                                                    : 'bg-black/20 border-white/5 text-slate-400 hover:text-slate-200 hover:border-white/10'
                                                }`}
                                        >
                                            <h4 className="text-sm font-medium text-slate-100">{tpl.name}</h4>
                                            <p className="text-[10px] opacity-75 truncate mt-1">{tpl.description}</p>
                                            <button
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    setTemplates(templates.filter(t => t.id !== tpl.id));
                                                    if (selectedTemplate.id === tpl.id) setSelectedTemplate(templates[0]);
                                                }}
                                                className="absolute top-2 right-2 text-slate-600 hover:text-ruby-400 p-1 opacity-0 group-hover:opacity-100 transition-all"
                                            >
                                                <Trash2 className="w-3.5 h-3.5" />
                                            </button>
                                        </div>
                                    ))}
                                </div>
                            </div>
                        </div>

                        {/* Right: Dynamic Template Editor */}
                        <div className="flex-1 overflow-y-auto bg-[#08090c] p-6 flex flex-col relative">
                            <div className="max-w-3xl w-full mx-auto space-y-6">
                                <div className="glass-panel p-6 rounded-xl space-y-4">
                                    <div className="flex justify-between items-start">
                                        <div>
                                            <span className="text-xs font-mono uppercase tracking-widest text-slate-500 font-bold block mb-1">Editing Template Blueprint</span>
                                            <input
                                                type="text"
                                                value={editedTemplateName}
                                                onChange={e => setEditedTemplateName(e.target.value)}
                                                disabled={selectedTemplate.category === 'default'}
                                                className="bg-transparent text-xl font-outfit font-bold text-white border-b border-transparent focus:border-violet-500/40 focus:outline-none w-96 py-1"
                                            />
                                        </div>
                                        {selectedTemplate.category === 'default' ? (
                                            <button
                                                onClick={() => handleDuplicateTemplate(selectedTemplate)}
                                                className="px-3 py-1.5 text-xs bg-violet-600 hover:bg-violet-500 rounded text-white font-medium flex items-center gap-1.5 transition-all shadow-[0_0_10px_rgba(139,92,246,0.3)]"
                                            >
                                                <CopyIcon className="w-3.5 h-3.5" /> Duplicate to Custom
                                            </button>
                                        ) : (
                                            <button
                                                onClick={handleSaveWorkflow}
                                                className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 rounded text-white text-xs font-bold flex items-center gap-1.5 transition-all shadow-[0_0_10px_rgba(16,185,129,0.3)]"
                                            >
                                                <Save className="w-4 h-4" /> Save Template
                                            </button>
                                        )}
                                    </div>

                                    <div>
                                        <label className="text-xs font-mono text-slate-400 uppercase tracking-widest block mb-1.5">Description</label>
                                        <textarea
                                            value={editedDescription}
                                            onChange={e => setEditedDescription(e.target.value)}
                                            disabled={selectedTemplate.category === 'default'}
                                            rows={2}
                                            className="w-full bg-black/40 border border-white/5 rounded p-2 text-sm text-slate-300 focus:outline-none focus:border-violet-500/40 resize-none"
                                        />
                                    </div>
                                </div>

                                {/* Interactive DAG steps */}
                                <div className="space-y-3">
                                    <h3 className="font-outfit text-sm font-semibold text-slate-400 uppercase tracking-widest mb-4">Orchestrator Sequence Graph</h3>

                                    <div className="space-y-3 relative before:absolute before:inset-0 before:ml-[1.4rem] before:h-full before:w-[2px] before:bg-white/5">
                                        {editedSteps.map((step, idx) => (
                                            <div key={step.id} className="relative flex items-center gap-4 group">
                                                <div className="w-11 h-11 rounded-full border border-white/10 bg-black/40 flex items-center justify-center font-mono text-xs text-slate-500 z-10 shrink-0">
                                                    {idx + 1}
                                                </div>

                                                <div className="flex-1 glass-panel p-4 rounded-xl flex items-center justify-between">
                                                    <div className="space-y-1">
                                                        <span className="text-[10px] font-mono uppercase tracking-widest text-violet-400 font-bold block">{step.type}</span>
                                                        <input
                                                            type="text"
                                                            defaultValue={step.title}
                                                            onChange={e => {
                                                                const copy = [...editedSteps];
                                                                copy[idx].title = e.target.value;
                                                                setEditedSteps(copy);
                                                            }}
                                                            disabled={selectedTemplate.category === 'default'}
                                                            className="bg-transparent border-b border-transparent focus:border-violet-500/30 font-medium text-slate-200 text-sm focus:outline-none py-0.5"
                                                        />
                                                    </div>

                                                    <div className="flex items-center gap-4">
                                                        {step.type !== 'gate' && (
                                                            <select
                                                                defaultValue={step.agent || (step.agents ? step.agents[0] : 'claude-sys-1')}
                                                                disabled={selectedTemplate.category === 'default'}
                                                                onChange={e => {
                                                                    const copy = [...editedSteps];
                                                                    if (copy[idx].agent) copy[idx].agent = e.target.value;
                                                                    setEditedSteps(copy);
                                                                }}
                                                                className="bg-black/60 border border-white/10 rounded px-2 py-1 text-xs text-slate-300 focus:outline-none"
                                                            >
                                                                <option value="claude-sys-1">claude-sys-1</option>
                                                                <option value="opencode-alpha">opencode-alpha</option>
                                                                <option value="hermes-worker">hermes-worker</option>
                                                            </select>
                                                        )}

                                                        {selectedTemplate.category === 'custom' && (
                                                            <button
                                                                onClick={() => deleteWorkflowStep(step.id)}
                                                                className="text-slate-600 hover:text-ruby-400 p-1 transition-all"
                                                            >
                                                                <X className="w-4 h-4" />
                                                            </button>
                                                        )}
                                                    </div>
                                                </div>
                                            </div>
                                        ))}

                                        {selectedTemplate.category === 'custom' && (
                                            <button
                                                onClick={addWorkflowStep}
                                                className="ml-16 py-3 px-4 border border-dashed border-white/10 hover:border-violet-500/40 rounded-xl text-xs text-slate-500 hover:text-white hover:bg-violet-500/5 transition-all flex items-center gap-2"
                                            >
                                                <Plus className="w-4 h-4" /> Insert Execution Step Node
                                            </button>
                                        )}
                                    </div>
                                </div>
                            </div>
                        </div>
                    </div>
                ) : (
                    <div className="flex-1 grid grid-cols-2 gap-8 p-8 overflow-y-auto">
                        {/* Left: Provider Register Form */}
                        <div className="space-y-6">
                            <div>
                                <h2 className="text-xl font-outfit font-bold text-white mb-2">Connect Git Hosting Provider</h2>
                                <p className="text-sm text-slate-400">Securely link external code hosts so that Demeteo agents can check out branches and draft code.</p>
                            </div>

                            <div className="glass-panel p-6 rounded-xl space-y-4">
                                <div>
                                    <label className="text-xs font-mono text-slate-400 uppercase tracking-widest block mb-2">Hosting Provider</label>
                                    <div className="flex gap-2">
                                        <button
                                            onClick={() => { setProvType('github'); setProvHost('github.com'); }}
                                            className={`flex-1 p-3 border rounded-lg text-xs font-bold transition-all flex items-center justify-center gap-1.5 ${provType === 'github' ? 'bg-violet-500/10 border-violet-500/40 text-violet-300' : 'bg-black/40 border-white/5 text-slate-400'
                                                }`}
                                        >
                                            <GitCommit className="w-4 h-4" /> GitHub
                                        </button>
                                        <button
                                            onClick={() => { setProvType('gitlab'); setProvHost('gitlab.example.com'); }}
                                            className={`flex-1 p-3 border rounded-lg text-xs font-bold transition-all flex items-center justify-center gap-1.5 ${provType === 'gitlab' ? 'bg-violet-500/10 border-violet-500/40 text-violet-300' : 'bg-black/40 border-white/5 text-slate-400'
                                                }`}
                                        >
                                            <GitMerge className="w-4 h-4" /> GitLab
                                        </button>
                                    </div>
                                </div>

                                <div>
                                    <label className="text-xs font-mono text-slate-400 uppercase tracking-widest block mb-1.5">Workspace Alias Name</label>
                                    <input
                                        type="text"
                                        value={provName}
                                        onChange={e => setProvName(e.target.value)}
                                        placeholder="e.g. Acme Enterprise Cloud"
                                        className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-sm text-white focus:outline-none focus:border-violet-500/50"
                                    />
                                </div>

                                <div>
                                    <label className="text-xs font-mono text-slate-400 uppercase tracking-widest block mb-1.5">Host URL</label>
                                    <input
                                        type="text"
                                        value={provHost}
                                        onChange={e => setProvHost(e.target.value)}
                                        placeholder="e.g. github.com"
                                        className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-sm text-white focus:outline-none focus:border-violet-500/50"
                                    />
                                </div>

                                <div>
                                    <label className="text-xs font-mono text-slate-400 uppercase tracking-widest block mb-1.5">Personal Access Token (PAT)</label>
                                    <div className="relative">
                                        <Key className="w-4 h-4 text-slate-500 absolute left-3 top-3.5" />
                                        <input
                                            type="password"
                                            value={provPat}
                                            onChange={e => setProvPat(e.target.value)}
                                            placeholder="ghp_************************************"
                                            className="w-full bg-black/40 border border-white/10 rounded-lg py-2.5 pl-9 pr-4 text-sm text-white focus:outline-none focus:border-violet-500/50"
                                        />
                                    </div>
                                </div>

                                <div className="pt-2">
                                    <button
                                        onClick={handleTestAndConnect}
                                        disabled={isTestingConn || !provName || !provPat}
                                        className={`w-full py-3 rounded-lg text-xs font-bold transition-all flex items-center justify-center gap-2 ${isTestingConn ? 'bg-violet-600/50 text-slate-300' : 'bg-violet-600 hover:bg-violet-500 text-white shadow-[0_0_15px_rgba(139,92,246,0.3)]'
                                            }`}
                                    >
                                        {isTestingConn ? (
                                            <>
                                                <RefreshCw className="w-4 h-4 animate-spin text-white" /> Connecting and Testing Auth PAT...
                                            </>
                                        ) : testSuccess ? (
                                            <>
                                                <Check className="w-4 h-4 text-emerald-400" /> Authorized Successfully!
                                            </>
                                        ) : (
                                            <>
                                                <CheckCircle2 className="w-4 h-4" /> Authenticate & Save Provider
                                            </>
                                        )}
                                    </button>
                                </div>
                            </div>
                        </div>

                        {/* Right: Active Connections Registry (Journey 2 Validation) */}
                        <div className="space-y-6">
                            <div>
                                <h2 className="text-xl font-outfit font-bold text-white mb-2">Connected Providers</h2>
                                <p className="text-sm text-slate-400">Current verified endpoints mapped to git resources.</p>
                            </div>

                            <div className="space-y-4">
                                {providers.map(prov => (
                                    <div key={prov.id} className="glass-panel p-4 rounded-xl flex items-center justify-between border-l-2 border-l-emerald-500">
                                        <div className="flex items-center gap-4">
                                            <img
                                                src={prov.avatar}
                                                alt="Workspace profile avatar placeholder"
                                                className="w-10 h-10 rounded-full object-cover border border-white/10"
                                            />
                                            <div>
                                                <h4 className="text-sm font-medium text-white">{prov.name}</h4>
                                                <div className="flex gap-2 text-[10px] font-mono text-slate-500 mt-1">
                                                    <span>User: <strong className="text-slate-300">@{prov.user}</strong></span>
                                                    <span>&bull;</span>
                                                    <span>Host: <strong className="text-slate-300">{prov.host}</strong></span>
                                                </div>
                                            </div>
                                        </div>

                                        <div className="flex items-center gap-3">
                                            <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">
                                                ACTIVE GATEWAY
                                            </span>
                                            <button
                                                onClick={() => setProviders(providers.filter(p => p.id !== prov.id))}
                                                className="text-slate-600 hover:text-ruby-400 p-1.5 rounded hover:bg-white/5 transition-all"
                                            >
                                                <Trash2 className="w-4 h-4" />
                                            </button>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
};

/* Custom Helper Icon component for copy/duplicate utility */
const CopyIcon = ({ className }) => (
    <svg
        className={className}
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
    >
        <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </svg>
);

export default function DemeteoApp() {
    const [projects, setProjects] = useState(MOCK_PROJECTS);
    const [currentProjectId, setCurrentProjectId] = useState('p1');
    const [activeFeatureId, setActiveFeatureId] = useState('f-8a7b9c');
    const [view, setView] = useState('home');
    const [activeTab, setActiveTab] = useState('workflows');

    const activeProject = projects.find(p => p.id === currentProjectId) || projects[0];

    return (
        <>
            <InjectStyles />
            <div className="flex h-screen w-full bg-[#08090c] overflow-hidden selection:bg-violet-500/30">
                <Sidebar projects={projects} currentProject={currentProjectId} setCurrentProject={setCurrentProjectId} setView={setView} />

                <div className="flex-1 flex flex-col min-w-0 relative">
                    <TopBar setView={setView} activeTab={activeTab} setActiveTab={setActiveTab} />

                    {/* View Dispatcher Routing */}
                    {view === 'home' && <ProjectHome setView={setView} activeProject={activeProject} setActiveFeatureId={setActiveFeatureId} />}
                    {view === 'detail' && <FeatureDetail setView={setView} featureId={activeFeatureId} />}
                    {view === 'gate' && <GateView setView={setView} />}
                    {view === 'new-project' && <NewProjectView setView={setView} setProjects={setProjects} setCurrentProjectId={setCurrentProjectId} />}
                    {view === 'control-panel' && <ControlPanel setView={setView} activeTab={activeTab} setActiveTab={setActiveTab} />}
                </div>
            </div>
        </>
    );
}