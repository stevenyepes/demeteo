import React, { useState } from 'react';
import { 
  TerminalSquare, Bot, Server, Settings, 
  Terminal, Cpu, Activity, X, Send,
  MemoryStick, Eye, AlertCircle, Check,
  Plus, Edit2, Key, Trash2, FileCode2, Maximize2,
  GitBranch, ShieldAlert, ChevronRight, CheckCircle2, CircleDashed
} from 'lucide-react';

interface MachineMock {
  id: string;
  name: string;
  type: string;
  user: string;
  status: string;
  agents: string[];
}

interface EnvFormState {
  id: string;
  name: string;
  type: string;
  user: string;
  status?: string;
  agents: string[];
}

const AppMockup = () => {
  const [machinesList, setMachinesList] = useState<MachineMock[]>([
    { id: 'm1', name: 'prod-db-cluster', type: 'server', user: 'root@10.0.5.12', status: 'connected', agents: ['OpenCode', 'Claude Code'] },
    { id: 'm2', name: 'staging-api', type: 'server', user: 'admin@192.168.1.5', status: 'offline', agents: ['Hermes'] },
    { id: 'm3', name: 'local-macbook', type: 'local', user: 'dev@localhost', status: 'connected', agents: ['Claude Code', 'Hermes'] }
  ]);
  const [activeMachine, setActiveMachine] = useState<MachineMock>(machinesList[0]);
  const [showMachineSelector, setShowMachineSelector] = useState(false);

  // Modal States
  const [isEnvModalOpen, setIsEnvModalOpen] = useState(false);
  const [envForm, setEnvForm] = useState<EnvFormState>({ id: '', name: '', type: 'server', user: '', status: 'offline', agents: [] });
  
  const [isNewThreadModalOpen, setIsNewThreadModalOpen] = useState(false);
  const [threadForm, setThreadForm] = useState({ name: '', type: 'worktree' }); // worktree | adhoc

  const [activeThread, setActiveThread] = useState('t1');
  const [workspaceMode, setWorkspaceMode] = useState('supervisor'); // 'supervisor' | 'terminal'
  const [inspectedFile, setInspectedFile] = useState<any>(null);

  const mockThreads = [
    { id: 't1', title: 'Implement OAuth2', type: 'worktree', branch: 'feature/agent-oauth', status: 'pending_approval' },
    { id: 't2', title: 'Analyze syslog memory leak', type: 'adhoc', status: 'idle' },
    { id: 't3', title: 'Update Dockerfile', type: 'worktree', branch: 'feature/docker-fix', status: 'running' }
  ];

  const workingMemory = [
    { name: 'src/main.rs', lines: 142, type: 'rust' },
    { name: 'Cargo.toml', lines: 34, type: 'toml' },
    { name: 'src/oauth.rs', lines: 89, type: 'rust', isNew: true }
  ];

  const handleMachineSelect = (m: MachineMock) => {
    setActiveMachine(m);
    setShowMachineSelector(false);
  };

  const openAddEnv = () => {
    setEnvForm({ id: '', name: '', type: 'server', user: '', status: 'offline', agents: [] });
    setIsEnvModalOpen(true);
    setShowMachineSelector(false);
  };

  const openEditEnv = (m: MachineMock, e: React.MouseEvent) => {
    if (e) e.stopPropagation();
    setEnvForm({ ...m });
    setIsEnvModalOpen(true);
    setShowMachineSelector(false);
  };

  const deleteEnv = (id: string, e: React.MouseEvent) => {
    if (e) e.stopPropagation();
    const updatedList = machinesList.filter(m => m.id !== id);
    setMachinesList(updatedList);
    if (activeMachine.id === id && updatedList.length > 0) {
      handleMachineSelect(updatedList[0]);
    }
    setIsEnvModalOpen(false);
  };

  const saveEnv = () => {
    if (envForm.id) {
      setMachinesList(machinesList.map(m => m.id === envForm.id ? { ...m, ...envForm, status: m.status } : m));
      if (activeMachine.id === envForm.id) {
        setActiveMachine({ ...activeMachine, ...envForm });
      }
    } else {
      const newEnv: MachineMock = {
        id: `m${Date.now()}`,
        name: envForm.name || 'unnamed-env',
        type: envForm.type,
        user: envForm.user || 'root@localhost',
        status: 'offline',
        agents: envForm.agents
      };
      setMachinesList([...machinesList, newEnv]);
    }
    setIsEnvModalOpen(false);
  };

  const toggleAgent = (agentName: string) => {
    setEnvForm(prev => {
      const agents = prev.agents || [];
      if (agents.includes(agentName)) return { ...prev, agents: agents.filter(a => a !== agentName) };
      return { ...prev, agents: [...agents, agentName] };
    });
  };

  const createThread = () => {
    // In a real app, this would append to the threads array and select it
    setIsNewThreadModalOpen(false);
    setThreadForm({ name: '', type: 'worktree' });
  };

  return (
    <div className="flex h-screen bg-[#050508] text-slate-300 font-sans selection:bg-cyan-500/30">
      
      {/* LEFT SIDEBAR: Environment & Threads */}
      <div className="w-64 bg-[#0a0a0e] border-r border-white/5 flex flex-col z-30 shadow-xl">
        
        {/* Environment Selector Dropdown */}
        <div className="p-4 border-b border-white/5 relative">
          <div className="text-[10px] uppercase tracking-wider text-slate-500 font-semibold mb-2 flex justify-between items-center">
            Target Environment
            <button onClick={openAddEnv} className="hover:text-cyan-400 transition-colors" title="Add Environment">
              <Plus size={12} />
            </button>
          </div>
          <div className="relative">
            <button 
              onClick={() => setShowMachineSelector(!showMachineSelector)}
              className="w-full bg-slate-900/80 border border-white/10 hover:border-white/20 rounded-xl p-3 flex items-center justify-between transition-all"
            >
              <div className="flex items-center">
                {activeMachine.type === 'local' ? (
                  <TerminalSquare size={16} className="text-emerald-400 mr-3" />
                ) : (
                  <Server size={16} className="text-cyan-400 mr-3" />
                )}
                <div className="text-left">
                  <div className="text-sm font-medium text-slate-200">{activeMachine.name}</div>
                  <div className="text-[10px] text-slate-500 font-mono mt-0.5">{activeMachine.user}</div>
                </div>
              </div>
              <Activity size={14} className={activeMachine.status === 'connected' ? 'text-emerald-500' : 'text-slate-600'} />
            </button>

            {/* Dropdown Menu */}
            {showMachineSelector && (
              <div className="absolute top-full left-0 w-full mt-2 bg-slate-900 border border-white/10 rounded-xl shadow-2xl z-50 overflow-hidden">
                <div className="max-h-64 overflow-y-auto">
                  {machinesList.map(m => (
                    <div 
                      key={m.id}
                      onClick={() => handleMachineSelect(m)}
                      className="p-3 hover:bg-white/5 cursor-pointer border-b border-white/5 last:border-0 flex items-center justify-between group"
                    >
                      <div className="flex items-center">
                        {m.type === 'local' ? <TerminalSquare size={14} className="text-slate-500 mr-2" /> : <Server size={14} className="text-slate-500 mr-2" />}
                        <div>
                          <div className="text-sm text-slate-200 font-medium">{m.name}</div>
                          <div className="text-[10px] text-slate-500 font-mono">{m.user}</div>
                        </div>
                      </div>
                      <div className="flex items-center space-x-1">
                        <button onClick={(e) => openEditEnv(m, e)} className="p-1.5 rounded-md text-slate-500 hover:text-cyan-400 hover:bg-cyan-500/10 transition-all opacity-0 group-hover:opacity-100"><Edit2 size={14} /></button>
                        <button onClick={(e) => deleteEnv(m.id, e)} className="p-1.5 rounded-md text-slate-500 hover:text-red-400 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"><Trash2 size={14} /></button>
                        {m.id === activeMachine.id && <Check size={14} className="text-emerald-500 ml-1" />}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>

        {/* Scrollable Context Areas */}
        <div className="flex-1 overflow-y-auto p-4 space-y-8">
          
          {/* Active Threads Block */}
          <section>
            <div className="flex items-center justify-between text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
              <div className="flex items-center">
                <Bot size={14} className="mr-2 text-slate-500" /> Active Threads
              </div>
              <button onClick={() => setIsNewThreadModalOpen(true)} className="p-1 rounded hover:bg-white/10 hover:text-cyan-400 transition-colors">
                <Plus size={14} />
              </button>
            </div>
            <div className="space-y-1.5">
              {mockThreads.map(thread => (
                <div 
                  key={thread.id} 
                  onClick={() => setActiveThread(thread.id)}
                  className={`p-2.5 rounded-lg cursor-pointer transition-all border ${
                    activeThread === thread.id 
                      ? 'bg-cyan-500/10 border-cyan-500/30 shadow-[0_0_15px_rgba(6,182,212,0.1)]' 
                      : 'bg-white/5 border-white/5 hover:bg-white/10'
                  }`}
                >
                  <div className="flex items-center justify-between mb-1">
                    <span className={`text-xs font-medium ${activeThread === thread.id ? 'text-cyan-400' : 'text-slate-300'}`}>
                      {thread.title}
                    </span>
                    {thread.status === 'pending_approval' && <span className="w-2 h-2 rounded-full bg-amber-500 animate-pulse"></span>}
                    {thread.status === 'running' && <span className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse"></span>}
                  </div>
                  <div className="flex items-center text-[10px] text-slate-500 font-mono">
                    {thread.type === 'worktree' ? (
                      <><GitBranch size={10} className="mr-1" /> {thread.branch}</>
                    ) : (
                      <><Terminal size={10} className="mr-1" /> Ad-Hoc Session</>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </section>

          {/* Working Memory Block */}
          <section>
            <div className="flex items-center justify-between text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
              <div className="flex items-center">
                <MemoryStick size={14} className="mr-2 text-slate-500" /> Working Memory
              </div>
              <span className="text-[10px] bg-slate-800 px-1.5 py-0.5 rounded text-cyan-500 font-mono">4.2k tkns</span>
            </div>
            <div className="space-y-1.5">
              {workingMemory.map((file, idx) => (
                <div 
                  key={idx} 
                  onClick={() => { setWorkspaceMode('supervisor'); setInspectedFile(file); }}
                  className={`flex items-center justify-between p-2 rounded-md border cursor-pointer transition-all ${
                    inspectedFile?.name === file.name ? 'bg-white/10 border-white/20' : 'bg-white/5 border-white/5 hover:border-white/10'
                  }`}
                >
                  <div className="flex items-center text-xs text-slate-300">
                    <FileCode2 size={12} className={`mr-2 ${inspectedFile?.name === file.name ? 'text-cyan-400' : 'text-slate-500'}`} />
                    <span className={inspectedFile?.name === file.name ? 'text-cyan-400 font-medium' : ''}>{file.name}</span>
                  </div>
                  <div className="flex items-center">
                    {file.isNew && <span className="text-[9px] text-emerald-400 mr-2 bg-emerald-500/10 px-1 rounded">NEW</span>}
                    <div className="text-[10px] text-slate-500 font-mono">{file.lines}L</div>
                  </div>
                </div>
              ))}
            </div>
          </section>
        </div>
      </div>

      {/* CENTER PANEL: The Orchestrator Stream & Inspector */}
      <div className="flex-1 flex flex-col min-w-0 bg-slate-950/40 relative shadow-2xl z-20">
        
        {/* Header Tabs */}
        <div className="h-14 border-b border-white/5 bg-[#0a0a0e]/50 backdrop-blur-md flex items-center justify-between px-6 z-10">
          <div className="flex items-center space-x-1 bg-slate-900/80 p-1 rounded-lg border border-white/5">
            <button 
              onClick={() => { setWorkspaceMode('supervisor'); }}
              className={`px-4 py-1.5 rounded-md text-xs font-medium transition-all flex items-center ${
                workspaceMode === 'supervisor' ? 'bg-cyan-500/20 text-cyan-400 shadow-sm' : 'text-slate-400 hover:text-slate-200'
              }`}
            >
              <Activity size={14} className="mr-2" /> Supervisor Plane
            </button>
            <button 
              onClick={() => { setWorkspaceMode('terminal'); setInspectedFile(null); }}
              className={`px-4 py-1.5 rounded-md text-xs font-medium transition-all flex items-center ${
                workspaceMode === 'terminal' ? 'bg-slate-800 text-white shadow-sm' : 'text-slate-400 hover:text-slate-200'
              }`}
            >
              <Terminal size={14} className="mr-2" /> Terminal
            </button>
          </div>
          
          <div className="flex items-center space-x-4">
            <div className="flex items-center text-xs font-mono text-emerald-400 bg-emerald-500/10 px-2 py-1 rounded border border-emerald-500/20">
              <ShieldAlert size={12} className="mr-1.5" /> Proxy Active
            </div>
            <button className="text-slate-400 hover:text-white transition-colors"><Settings size={16} /></button>
          </div>
        </div>

        {/* Dynamic Workspace Container */}
        <div className="flex-1 flex min-h-0 overflow-hidden">
          
          {/* Main Stream (Shrinks if Inspector is open) */}
          <div className={`flex flex-col min-w-0 transition-all duration-300 ${inspectedFile && workspaceMode === 'supervisor' ? 'w-1/2 border-r border-white/5' : 'w-full'}`}>
            
            {workspaceMode === 'supervisor' ? (
              <>
                {/* Event Stream (No Chat Bubbles, pure logs and diffs) */}
                <div className="flex-1 overflow-y-auto p-6 space-y-6 scroll-smooth font-mono text-sm">
                  
                  {/* Human Directive */}
                  <div className="flex items-start text-slate-300">
                    <span className="text-cyan-500 mr-3 mt-0.5 font-bold">{'>'}</span>
                    <div className="leading-relaxed">Set up the initial OAuth2 routes using Actix-web in src/oauth.rs and update Cargo.toml</div>
                  </div>

                  {/* Agent Auto-Approved Actions */}
                  <div className="pl-6 border-l-2 border-slate-800 space-y-3 py-2">
                    <div className="flex items-center text-slate-500 text-xs">
                      <CheckCircle2 size={12} className="mr-2 text-slate-600" />
                      <span className="text-slate-600 mr-2">[Auto-Approved]</span> Agent executed <code className="bg-white/5 px-1 rounded ml-1 text-slate-400">git status</code>
                    </div>
                    <div className="flex items-center text-slate-500 text-xs">
                      <CheckCircle2 size={12} className="mr-2 text-slate-600" />
                      <span className="text-slate-600 mr-2">[Auto-Approved]</span> Agent executed <code className="bg-white/5 px-1 rounded ml-1 text-slate-400">cat Cargo.toml</code>
                    </div>
                    <div className="flex items-center text-cyan-500/70 text-xs">
                      <CircleDashed size={12} className="mr-2 animate-spin-slow" />
                      <span>Agent synthesizing response...</span>
                    </div>
                  </div>

                  {/* Intercepted Write Action (Approval Queue) */}
                  <div className="mt-6 bg-[#0a0a0e] border border-amber-500/30 rounded-xl overflow-hidden shadow-lg shadow-amber-500/5">
                    <div className="px-4 py-2 bg-amber-500/10 border-b border-amber-500/20 flex items-center justify-between">
                      <div className="flex items-center text-xs font-semibold text-amber-500 tracking-wide uppercase">
                        <ShieldAlert size={14} className="mr-2" /> Intercepted Action: File Write
                      </div>
                      <div className="text-[10px] text-slate-500 flex items-center">
                        <Cpu size={10} className="mr-1 text-slate-500" /> Agent: OpenCode
                      </div>
                    </div>
                    <div className="p-4">
                      <div className="text-xs text-slate-400 mb-3 flex items-center">
                        <ChevronRight size={14} className="text-slate-600 mr-1" />
                        Target: <span className="font-mono text-cyan-400 ml-2">src/oauth.rs</span>
                      </div>
                      <div className="bg-[#050508] border border-white/5 rounded-lg p-4 font-mono text-[13px] leading-relaxed relative">
                        <div className="absolute top-2 right-2 text-[10px] bg-emerald-500/20 text-emerald-400 px-1.5 rounded">+ 42 lines</div>
                        <div className="text-emerald-400">+ use actix_web::{'{'}get, web, HttpResponse, Responder{'}'};</div>
                        <div className="text-emerald-400">+ use reqwest::Client;</div>
                        <div className="text-emerald-400">+</div>
                        <div className="text-emerald-400">+ #[get("/login")]</div>
                        <div className="text-emerald-400">+ pub async fn login() -&gt; impl Responder {'{'}</div>
                        <div className="text-emerald-400">+     HttpResponse::Ok().body("OAuth implementation pending")</div>
                        <div className="text-emerald-400">+ {'}'}</div>
                      </div>
                    </div>
                    <div className="px-4 py-3 bg-slate-950/80 flex items-center justify-between border-t border-white/5">
                      <button 
                        onClick={() => setInspectedFile({ name: 'src/oauth.rs', lines: '42', type: 'rust' })}
                        className="px-3 py-1.5 rounded-lg border border-white/10 text-xs text-slate-300 hover:bg-white/5 transition-colors flex items-center"
                      >
                        <Eye size={14} className="mr-1.5" /> Inspect Context
                      </button>
                      <div className="flex space-x-2">
                        <button className="px-4 py-1.5 rounded-lg border border-red-500/30 bg-red-500/10 text-red-400 text-xs hover:bg-red-500/20 transition-colors font-medium">
                          Reject
                        </button>
                        <button className="px-4 py-1.5 rounded-lg bg-cyan-500 text-slate-950 text-xs hover:bg-cyan-400 transition-colors shadow-[0_0_15px_rgba(6,182,212,0.4)] font-bold flex items-center">
                          <Check size={14} className="mr-1.5" /> Approve Execution
                        </button>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Supervisor Input Box (Directives) */}
                <div className="p-4 bg-slate-950/80 border-t border-white/5">
                  <div className="max-w-4xl mx-auto relative flex items-center">
                    <span className="absolute left-4 text-cyan-500 font-bold">{'>'}</span>
                    <input 
                      type="text" 
                      placeholder="Enter directive or feedback for the agent..." 
                      className="w-full bg-[#0a0a0e] border border-white/10 rounded-xl py-3 pl-10 pr-12 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50 focus:shadow-[0_0_15px_rgba(6,182,212,0.15)] transition-all font-mono placeholder-slate-600"
                    />
                    <button className="absolute right-2 p-2 rounded-lg bg-cyan-500/10 text-cyan-400 hover:bg-cyan-500 hover:text-slate-900 transition-all flex items-center justify-center">
                      <Send size={16} />
                    </button>
                  </div>
                  <div className="text-center mt-2 text-[10px] text-slate-600 font-mono">
                    Demeteo PDP Active • OpenCode Container Running
                  </div>
                </div>
              </>
            ) : (
              /* INTERACTIVE TERMINAL WORKSPACE */
              <div className="flex-1 bg-[#050508] p-4 font-mono text-[13px] text-slate-300 overflow-y-auto shadow-inner flex flex-col relative">
                <div className="text-emerald-500 mb-2">root@prod-db-cluster:~# <span className="text-slate-300">cd ~/demeteo-core</span></div>
                <div className="text-emerald-500 mb-2">root@prod-db-cluster:~/demeteo-core# <span className="text-slate-300">git status</span></div>
                <div className="text-slate-400 mb-4 whitespace-pre">
                  On branch main{'\n'}Your branch is up to date with 'origin/main'.{'\n\n'}nothing to commit, working tree clean
                </div>
                <div className="flex items-center">
                  <span className="text-emerald-500 mr-2">root@prod-db-cluster:~/demeteo-core#</span>
                  <span className="w-2 h-4 bg-slate-400 animate-pulse"></span>
                </div>
              </div>
            )}
          </div>

          {/* CODE INSPECTOR PANEL (Sliding Split View) */}
          {inspectedFile && workspaceMode === 'supervisor' && (
            <div className="w-1/2 flex flex-col bg-[#0a0a0e] border-l border-white/5 z-10 animate-in slide-in-from-right-8 duration-300 shadow-2xl">
              <div className="h-12 px-4 bg-[#050508] border-b border-white/5 flex items-center justify-between select-none">
                <div className="flex items-center space-x-3">
                  <FileCode2 size={14} className="text-cyan-500" />
                  <span className="text-xs font-mono text-slate-300">{inspectedFile.name}</span>
                  <span className="text-[9px] text-amber-500 border border-amber-500/20 px-1.5 py-0.5 rounded bg-amber-500/10 uppercase font-bold tracking-wider">Read-only</span>
                </div>
                <div className="flex items-center space-x-3 text-slate-500">
                  <button className="hover:text-cyan-400 transition-colors p-1" title="Open in External Editor"><Maximize2 size={14} /></button>
                  <button onClick={() => setInspectedFile(null)} className="hover:text-red-400 transition-colors p-1" title="Close Inspector"><X size={16} /></button>
                </div>
              </div>
              <div className="flex-1 overflow-y-auto font-mono text-[13px] leading-relaxed flex bg-[#0a0a0e]">
                <div className="w-12 bg-[#050508] border-r border-white/5 text-slate-600 text-right pr-3 py-4 select-none flex flex-col">
                  {[...Array(20)].map((_, i) => (
                    <span key={i} className={i >= 5 && i <= 10 ? "text-cyan-600 font-bold bg-cyan-500/5 -mr-3 pr-3" : ""}>{i + 1}</span>
                  ))}
                </div>
                <div className="flex-1 py-4 px-4 text-slate-300 overflow-x-auto whitespace-pre">
                  <div><span className="text-slate-500 italic">// OAuth Implementation Proposal</span></div>
                  <div className="bg-emerald-500/5 -mx-4 px-4 py-1 border-l-2 border-emerald-500 shadow-inner mt-2">
                    <span className="text-emerald-400">use</span> actix_web::{'{'}get, web, HttpResponse, Responder{'}'};<br/>
                    <span className="text-emerald-400">use</span> reqwest::Client;<br/>
                    <br/>
                    <span className="text-emerald-400">#[get("/login")]</span><br/>
                    <span className="text-emerald-400">pub async fn</span> <span className="text-blue-300">login</span>() -&gt; <span className="text-emerald-400">impl</span> Responder {'{'}<br/>
                    {"    "}<span className="text-blue-300">HttpResponse::Ok</span>().body(<span className="text-green-300">"OAuth implementation pending"</span>)<br/>
                    {'}'}
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* MODAL: New Thread (Dual-Mode Execution) */}
      {isNewThreadModalOpen && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
          <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-md shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
            <div className="px-6 py-4 border-b border-white/5 flex justify-between items-center bg-[#050508]">
              <h3 className="text-sm font-semibold text-white flex items-center">
                <Bot size={16} className="mr-2 text-cyan-400" /> Initialize Agent Thread
              </h3>
              <button onClick={() => setIsNewThreadModalOpen(false)} className="text-slate-500 hover:text-white transition-colors"><X size={16} /></button>
            </div>
            
            <div className="p-6 space-y-5">
              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Thread Objective</label>
                <input 
                  type="text" 
                  value={threadForm.name}
                  onChange={e => setThreadForm({...threadForm, name: e.target.value})}
                  placeholder="e.g., Fix Redis connection timeout..." 
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50" 
                />
              </div>

              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-2 font-semibold">Execution Sandbox Mode</label>
                <div className="grid grid-cols-2 gap-3">
                  <button 
                    onClick={() => setThreadForm({...threadForm, type: 'worktree'})}
                    className={`p-3 rounded-lg border text-left transition-all flex flex-col ${
                      threadForm.type === 'worktree' 
                        ? 'bg-cyan-500/10 border-cyan-500/50 shadow-[0_0_15px_rgba(6,182,212,0.15)]' 
                        : 'bg-[#050508] border-white/5 hover:border-white/10'
                    }`}
                  >
                    <div className={`flex items-center text-xs font-semibold mb-1 ${threadForm.type === 'worktree' ? 'text-cyan-400' : 'text-slate-300'}`}>
                      <GitBranch size={14} className="mr-1.5" /> Project Mode
                    </div>
                    <div className="text-[10px] text-slate-500 leading-tight">Isolates agent in a secure Git Worktree branch.</div>
                  </button>
                  
                  <button 
                    onClick={() => setThreadForm({...threadForm, type: 'adhoc'})}
                    className={`p-3 rounded-lg border text-left transition-all flex flex-col ${
                      threadForm.type === 'adhoc' 
                        ? 'bg-violet-500/10 border-violet-500/50 shadow-[0_0_15px_rgba(139,92,246,0.15)]' 
                        : 'bg-[#050508] border-white/5 hover:border-white/10'
                    }`}
                  >
                    <div className={`flex items-center text-xs font-semibold mb-1 ${threadForm.type === 'adhoc' ? 'text-violet-400' : 'text-slate-300'}`}>
                      <Terminal size={14} className="mr-1.5" /> Ad-Hoc Mode
                    </div>
                    <div className="text-[10px] text-slate-500 leading-tight">Direct directory access via Permission Proxy.</div>
                  </button>
                </div>
              </div>
            </div>

            <div className="px-6 py-4 border-t border-white/5 bg-[#050508] flex justify-end space-x-3">
              <button onClick={() => setIsNewThreadModalOpen(false)} className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors">Cancel</button>
              <button onClick={createThread} className="px-5 py-2 rounded-lg text-xs font-bold bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all flex items-center">
                Launch Thread <ChevronRight size={14} className="ml-1" />
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Environment Config Modal */}
      {isEnvModalOpen && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
          <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-md shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
            <div className="px-6 py-4 border-b border-white/5 flex justify-between items-center bg-[#050508]">
              <h3 className="text-sm font-semibold text-white flex items-center">
                <Server size={16} className="mr-2 text-cyan-400" /> Configure Environment
              </h3>
              <button onClick={() => setIsEnvModalOpen(false)} className="text-slate-500 hover:text-white transition-colors"><X size={16} /></button>
            </div>
            
            <div className="p-6 space-y-4">
              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Environment Name</label>
                <input 
                  type="text" 
                  value={envForm.name}
                  onChange={e => setEnvForm({...envForm, name: e.target.value})}
                  placeholder="e.g., prod-db-cluster" 
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50" 
                />
              </div>

              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Connection Details (User@Host)</label>
                <input 
                  type="text" 
                  value={envForm.user}
                  onChange={e => setEnvForm({...envForm, user: e.target.value})}
                  placeholder="e.g., root@10.0.5.12" 
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50" 
                />
              </div>

              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-2 font-semibold">Type</label>
                  <div className="space-y-2">
                    <label className="flex items-center text-xs text-slate-300 cursor-pointer">
                      <input 
                        type="radio" 
                        name="envType" 
                        value="server"
                        checked={envForm.type === 'server'}
                        onChange={() => setEnvForm({...envForm, type: 'server'})}
                        className="mr-2 accent-cyan-500" 
                      />
                      Remote SSH Server
                    </label>
                    <label className="flex items-center text-xs text-slate-300 cursor-pointer">
                      <input 
                        type="radio" 
                        name="envType" 
                        value="local"
                        checked={envForm.type === 'local'}
                        onChange={() => setEnvForm({...envForm, type: 'local'})}
                        className="mr-2 accent-cyan-500" 
                      />
                      Local Node
                    </label>
                  </div>
                </div>

                <div>
                  <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold flex items-center">
                    <Key size={10} className="mr-1" /> Auth Config
                  </label>
                  <div className="text-[11px] text-slate-400 bg-[#050508] border border-white/5 rounded-lg p-2 font-mono">
                    SSH Key (Default)
                  </div>
                </div>
              </div>

              <div className="border-t border-white/5 pt-4">
                <div className="flex items-center text-amber-500 text-[11px] bg-amber-500/10 p-2.5 rounded-lg border border-amber-500/20">
                  <AlertCircle size={14} className="mr-2 flex-shrink-0" />
                  <span>Ensure public key authentication is configured on the target.</span>
                </div>
              </div>

              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-2 font-semibold flex items-center">
                  <Cpu size={10} className="mr-1" /> Enabled Agents
                </label>
                <div className="flex flex-wrap gap-2">
                  {['Claude Code', 'OpenCode', 'Hermes'].map(agent => (
                    <button
                      key={agent}
                      onClick={() => toggleAgent(agent)}
                      className={`px-3 py-1.5 rounded-lg border text-xs font-mono transition-all ${
                        envForm.agents?.includes(agent) 
                          ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-400' 
                          : 'bg-[#050508] border-white/5 text-slate-500 hover:border-white/10'
                      }`}
                    >
                      {agent}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            <div className="px-6 py-4 border-t border-white/5 bg-[#050508] flex justify-end space-x-3">
              <button onClick={() => setIsEnvModalOpen(false)} className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors">Cancel</button>
              <button onClick={saveEnv} className="px-5 py-2 rounded-lg text-xs font-bold bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all">
                Save Environment
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default AppMockup;
