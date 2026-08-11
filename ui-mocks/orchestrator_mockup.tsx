import React, { useState, useEffect, useRef } from 'react';
import { 
  Menu, Search, Settings, User, Server, TerminalSquare, 
  Play, StopCircle, Code, Eye, X, ChevronRight, CheckCircle2, CircleDashed, Check,
  ArrowLeft, Sparkles, GitPullRequest, Activity, FileText, RotateCcw, ChevronDown, ChevronUp,
  Plus, LayoutGrid, GitBranch, PlayCircle
} from 'lucide-react';

const App = () => {
  const [currentView, setCurrentView] = useState('dashboard');
  const [leftPaneWidth, setLeftPaneWidth] = useState(65); // Percentage
  const [isDragging, setIsDragging] = useState(false);
  const containerRef = useRef(null);
  
  const [selectedNode, setSelectedNode] = useState(null);
  const [isPromptExpanded, setIsPromptExpanded] = useState(false);
  const [activeTab, setActiveTab] = useState('timeline');

  // Mock projects for the sidebar
  const projects = [
    { id: 1, name: 'demeteo-remote', status: 'Ready', branchCount: 1, runCount: 1, active: true },
    { id: 2, name: 'spectacular', status: 'Ready', branchCount: 1, runCount: 0 },
    { id: 3, name: 'Demeteo', status: 'Ready', branchCount: 1, runCount: 1 },
    { id: 4, name: 'terraform dev cont...', status: 'Ready', branchCount: 1, runCount: 0 }
  ];

  const initialPrompt = "This is a simple web application, react with vite, its a modern nextjs app to create teams, keep an inventory of heroes, plan for pve and pvp, show the current meta for Arena and RTA. A space for updates to the game Epic Seven";

  // Simulated data for the graph
  const nodes = [
    { id: '1', title: 'Parse App Description', status: 'COMPLETED', type: 'system' },
    { id: '2', title: 'System Setup Validation', status: 'COMPLETED', type: 'system' },
    { id: '3', title: 'Generate Setup Tasks', status: 'COMPLETED', type: 'agent' },
    { id: '4', title: 'Plan Frontend Architecture', status: 'COMPLETED', type: 'agent' },
    { id: '5', title: 'Implement Tickets', status: 'RUNNING', type: 'agent', isSelected: true },
    { id: '6', title: 'Validate Routing Logic', status: 'QUEUED', type: 'system' },
    { id: '7', title: 'Review Code', status: 'QUEUED', type: 'agent' },
    { id: '8', title: 'Automated Browser QA', status: 'QUEUED', type: 'agent' }
  ];

  useEffect(() => {
    const handleMouseMove = (e) => {
      if (!isDragging || !containerRef.current) return;
      
      const containerRect = containerRef.current.getBoundingClientRect();
      // Calculate new width as a percentage of the container width
      const newWidthPercent = ((e.clientX - containerRect.left) / containerRect.width) * 100;
      
      // Constrain the width between 20% and 80%
      if (newWidthPercent > 20 && newWidthPercent < 80) {
        setLeftPaneWidth(newWidthPercent);
      }
    };

    const handleMouseUp = () => {
      setIsDragging(false);
    };

    if (isDragging) {
      document.addEventListener('mousemove', handleMouseMove);
      document.addEventListener('mouseup', handleMouseUp);
    } else {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    }

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDragging]);

  const handleMouseDown = () => {
    setIsDragging(true);
  };

  return (
    <div className="flex h-screen w-full bg-[#0d1117] text-gray-300 font-sans overflow-hidden">
      
      {/* Left Sidebar (Workspaces) */}
      <div className="w-64 bg-[#090c10] border-r border-[#30363d] flex flex-col z-10 shrink-0 h-full transition-all duration-300">
        
        {/* Header / Logo */}
        <div className="h-16 flex items-center px-5 gap-3 shrink-0 border-b border-transparent">
            <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-indigo-500 via-purple-500 to-blue-500 p-px shadow-sm">
                <div className="w-full h-full bg-[#090c10] rounded-[7px] flex items-center justify-center">
                    <div className="w-3 h-3 bg-gradient-to-tr from-indigo-400 to-blue-400 rounded-full blur-[1px]"></div>
                </div>
            </div>
            <span className="font-bold text-white text-[15px] tracking-wide">demeteo</span>
        </div>
        
        {/* Workspaces Header Actions */}
        <div className="px-5 py-4 flex items-center justify-between text-gray-500">
            <span className="text-[10px] font-bold tracking-wider uppercase">Workspaces</span>
            <div className="flex items-center gap-3">
                <Plus size={14} className="hover:text-gray-300 cursor-pointer transition-colors" />
                <Sparkles size={13} className="hover:text-gray-300 cursor-pointer transition-colors" />
                <LayoutGrid size={13} className="hover:text-gray-300 cursor-pointer transition-colors" />
            </div>
        </div>

        {/* Search / Filter */}
        <div className="px-3 pb-3">
            <div className="bg-[#0d1117] border border-[#30363d] rounded-lg flex items-center px-3 py-2 focus-within:border-gray-500 focus-within:ring-1 focus-within:ring-gray-500/20 transition-all">
                <Search size={14} className="text-gray-500 mr-2 shrink-0" />
                <input 
                    type="text" 
                    placeholder="Filter projects..." 
                    className="bg-transparent border-none text-xs text-gray-300 w-full focus:outline-none placeholder:text-gray-600" 
                />
            </div>
        </div>

        {/* Project List */}
        <div className="flex-1 overflow-auto custom-scrollbar px-2 space-y-1">
            {projects.map(p => (
                <div 
                    key={p.id} 
                    className={`flex items-center justify-between p-2.5 rounded-lg cursor-pointer transition-colors ${
                        p.active ? 'bg-[#1c2128] border border-[#30363d]/50 shadow-sm' : 'hover:bg-[#161b22] border border-transparent'
                    }`}
                >
                    <div className="flex items-start gap-3 overflow-hidden">
                        <div className="mt-1.5 w-1.5 h-1.5 rounded-full bg-green-500 shrink-0 shadow-[0_0_5px_rgba(34,197,94,0.5)]"></div>
                        <div className="flex flex-col truncate">
                            <span className={`text-sm truncate ${p.active ? 'text-gray-200 font-medium' : 'text-gray-400'}`}>
                                {p.name}
                            </span>
                            <span className="text-[10px] text-gray-500 mt-0.5">{p.status}</span>
                        </div>
                    </div>
                    <div className="flex items-center gap-3 text-gray-600 text-[10px] shrink-0 font-mono">
                        <div className="flex items-center gap-1" title="Branches"><GitBranch size={10} /> {p.branchCount}</div>
                        <div className="flex items-center gap-1" title="Runs"><PlayCircle size={10} /> {p.runCount}</div>
                    </div>
                </div>
            ))}
        </div>

        {/* Bottom Bar: Terminals & Shortcuts */}
        <div className="mt-auto border-t border-[#30363d] p-4 bg-[#0d1117]/50">
            <div className="flex items-center justify-between text-sm text-gray-400 hover:text-white cursor-pointer group mb-4">
                <div className="flex items-center gap-3 font-medium">
                    <TerminalSquare size={16} className="group-hover:text-teal-400 transition-colors" />
                    Terminals
                </div>
                <div className="w-5 h-5 rounded bg-[#161b22] border border-[#30363d] flex items-center justify-center text-[10px] font-bold text-gray-300">1</div>
            </div>
            <div className="text-[9px] text-gray-600 font-mono flex flex-col gap-1.5">
                <div className="flex items-center gap-2">
                    <span className="bg-[#1c2128] border border-[#30363d] px-1 rounded">⌘1-4</span> 
                    <span>to jump</span>
                </div>
                <div className="flex items-center gap-2">
                    <span className="bg-[#1c2128] border border-[#30363d] px-1 rounded">⌘K</span> 
                    <span>for palette</span>
                </div>
            </div>
        </div>
      </div>

      {currentView === 'dashboard' ? (
        <ProjectDashboard onOpenPipeline={() => setCurrentView('pipeline')} />
      ) : (
        <div className="flex-1 flex flex-col overflow-hidden relative">
          
          {/* Top Navbar */}
          <header className="h-16 border-b border-[#30363d] bg-[#0d1117] flex items-center justify-between px-4 sm:px-6 shrink-0 overflow-x-auto custom-scrollbar">
          <div className="flex items-center shrink-0">
            {/* Back Button Group */}
            <div className="flex items-center gap-2 pr-4 border-r border-[#30363d] mr-4 shrink-0">
              <button 
                onClick={() => setCurrentView('dashboard')}
                className="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-[#1c2128] text-gray-400 hover:text-white transition-colors group"
              >
                <ArrowLeft size={16} className="group-hover:-translate-x-0.5 transition-transform shrink-0" />
                <span className="text-xs font-semibold tracking-wider uppercase whitespace-nowrap">demeteo-remote</span>
              </button>
            </div>
            
            {/* Context Badges */}
            <div className="flex items-center gap-2 shrink-0">
              <div className="flex items-center gap-2 px-2.5 py-1 rounded-full bg-blue-500/10 border border-blue-500/20 text-blue-400">
                <div className="w-1.5 h-1.5 rounded-full bg-blue-400 animate-pulse shadow-[0_0_8px_rgba(96,165,250,0.8)]"></div>
                <span className="text-[10px] font-bold tracking-wider leading-none mt-px">RUNNING</span>
              </div>
              
              <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-full border border-[#30363d] bg-[#161b22] text-gray-400">
                <Server size={10} />
                <span className="text-[10px] font-bold tracking-wider leading-none mt-px">LOCAL</span>
              </div>
            </div>
          </div>
          
          <div className="flex items-center gap-3 text-sm shrink-0 pl-4">
             {/* Unified Metrics Bar */}
             <div className="flex items-center gap-5 mr-2 bg-[#161b22] px-4 py-1.5 rounded-lg border border-[#30363d] h-10">
                <MetricItem label="ELAPSED" value="10m 56s" />
                <MetricItem label="COST" value="$0.983" color="text-green-400" />
                <MetricItem label="TOKENS" value="600.7K" color="text-purple-400" />
             </div>

            <button className="whitespace-nowrap bg-teal-500/10 hover:bg-teal-500/20 text-teal-400 px-4 py-2 rounded-lg text-sm font-medium transition-colors border border-teal-500/20 flex items-center gap-2 h-10">
              <Code size={16} /> Code with Agent
            </button>
            <button className="whitespace-nowrap bg-purple-500/10 hover:bg-purple-500/20 text-purple-400 px-4 py-2 rounded-lg text-sm font-medium transition-colors border border-purple-500/20 flex items-center gap-2 h-10">
              <Eye size={16} /> Browse Code
            </button>
            <button className="whitespace-nowrap bg-red-500/10 hover:bg-red-500/20 text-red-400 px-4 py-2 rounded-lg text-sm font-medium transition-colors border border-red-500/20 flex items-center gap-2 h-10">
              <X size={16} /> Cancel Feature
            </button>
          </div>
        </header>

        {/* Workspace Area - Resizable Layout */}
        <div className="flex-1 flex flex-col p-4 overflow-hidden">
            
            {/* Feature Context Header (Merged Title & Prompt) */}
            <div className="mb-4 bg-[#161b22] border border-[#30363d] rounded-lg overflow-hidden shrink-0 shadow-sm transition-all duration-200">
                <div 
                    className="flex justify-between items-center px-5 py-4 cursor-pointer hover:bg-[#1c2128] transition-colors group"
                    onClick={() => setIsPromptExpanded(!isPromptExpanded)}
                >
                    <div className="flex items-center gap-4">
                        <div className="p-2.5 bg-blue-500/10 rounded-lg border border-blue-500/20 text-blue-400 group-hover:scale-105 transition-transform duration-300">
                            <Sparkles size={20} />
                        </div>
                        <div className="flex flex-col">
                            <h1 className="text-xl font-bold text-white tracking-wide">epic-builder</h1>
                            <span className="text-xs text-gray-500 mt-1 flex items-center gap-1">
                                {isPromptExpanded ? 'Hide' : 'View'} feature prompt <ChevronRight size={12} className={`transition-transform duration-200 ${isPromptExpanded ? 'rotate-90' : ''}`} />
                            </span>
                        </div>
                    </div>
                </div>
                
                {isPromptExpanded && (
                    <div className="px-5 pb-5 pt-1 border-t border-[#30363d] bg-[#0d1117]/30">
                        <div className="flex items-center gap-2 mb-3 mt-4">
                            <TerminalSquare size={14} className="text-gray-500" />
                            <span className="text-[10px] font-bold text-gray-500 tracking-wider uppercase">Original Instructions</span>
                        </div>
                        <div className="text-sm text-gray-300 leading-relaxed pl-5 border-l-2 border-[#30363d]">
                            {initialPrompt}
                        </div>
                    </div>
                )}
            </div>

            {/* Split Layout Container */}
            <div 
                ref={containerRef} 
                className="flex-1 flex w-full h-full relative border border-[#30363d] rounded-lg overflow-hidden bg-[#0d1117]"
            >
                {/* Left Pane - Graph View */}
                <div 
                    style={{ width: `${leftPaneWidth}%` }} 
                    className="h-full flex flex-col relative"
                >
                     {/* Internal Header for Left Pane */}
                    <div className="h-12 border-b border-[#30363d] bg-[#161b22] flex items-center px-4 gap-4 shrink-0">
                        <button 
                            className={`text-sm font-medium py-3 border-b-2 transition-colors ${activeTab === 'graph' ? 'text-teal-400 border-teal-400' : 'text-gray-500 hover:text-gray-300 border-transparent'}`}
                            onClick={() => setActiveTab('graph')}
                        >
                            Graph
                        </button>
                        <button 
                            className={`text-sm font-medium py-3 border-b-2 transition-colors ${activeTab === 'timeline' ? 'text-teal-400 border-teal-400' : 'text-gray-500 hover:text-gray-300 border-transparent'}`}
                            onClick={() => setActiveTab('timeline')}
                        >
                            Timeline
                        </button>
                    </div>

                    {/* Left Pane Content Area */}
                    {activeTab === 'graph' ? (
                        <div className="flex-1 relative overflow-auto custom-scrollbar p-8 flex justify-center bg-[#090c10] bg-[radial-gradient(ellipse_at_center,_var(--tw-gradient-stops))] from-[#161b22] via-[#090c10] to-[#090c10]">
                            <div className="absolute top-4 right-4 z-10">
                                 <button className="bg-[#1c2128] border border-[#30363d] hover:bg-[#30363d] text-xs px-3 py-1.5 rounded transition-colors text-gray-300">
                                     Auto-layout
                                 </button>
                            </div>

                            {/* Simulated Graph Nodes */}
                            <div className="flex flex-col items-center py-10 min-w-max gap-8 relative">
                                 {nodes.map((node, index) => (
                                    <GraphNode 
                                        key={node.id} 
                                        node={node} 
                                        isLast={index === nodes.length - 1} 
                                        onClick={() => setSelectedNode(node)}
                                    />
                                 ))}
                            </div>
                            
                            {/* Zoom Controls */}
                            <div className="absolute bottom-4 left-4 flex flex-col bg-[#161b22] border border-[#30363d] rounded-md overflow-hidden">
                                <button className="p-2 hover:bg-[#30363d] text-gray-400 border-b border-[#30363d]">+</button>
                                <button className="p-2 hover:bg-[#30363d] text-gray-400 border-b border-[#30363d]">-</button>
                                <button className="p-2 hover:bg-[#30363d] text-gray-400">[]</button>
                            </div>
                        </div>
                    ) : (
                        <TimelineView />
                    )}
                </div>

                {/* Resizer Handle */}
                <div 
                    className="w-1 cursor-col-resize hover:bg-teal-500/50 bg-[#30363d] z-20 flex flex-col justify-center items-center group transition-colors"
                    onMouseDown={handleMouseDown}
                >
                    <div className="h-8 w-1 bg-gray-500 rounded-full opacity-0 group-hover:opacity-100 transition-opacity"></div>
                </div>

                {/* Right Pane - Details Side Panel */}
                <div 
                    style={{ width: `${100 - leftPaneWidth}%` }} 
                    className="h-full bg-[#161b22] flex flex-col"
                >
                    {/* Details Panel Header */}
                    <div className="h-14 border-b border-[#30363d] flex items-center justify-between px-4 shrink-0 bg-[#161b22]">
                         <div className="flex items-center gap-3">
                             <div className="w-6 h-6 rounded bg-teal-500/20 text-teal-400 flex items-center justify-center">
                                 <Play size={12} fill="currentColor" />
                             </div>
                             <h2 className="text-sm font-bold tracking-wider text-white">IMPLEMENT TICKETS</h2>
                             <span className="text-xs bg-teal-500/20 text-teal-400 px-1.5 py-0.5 rounded border border-teal-500/30">RUNNING</span>
                         </div>
                         <button className="text-gray-500 hover:text-gray-300">
                             <X size={16} />
                         </button>
                    </div>

                    {/* Details Panel Tabs */}
                    <div className="flex border-b border-[#30363d] px-4 gap-6 shrink-0 bg-[#0d1117]">
                        <button className="text-teal-400 text-sm font-medium border-b-2 border-teal-400 py-2.5">Overview</button>
                        <button className="text-gray-500 hover:text-gray-300 text-sm py-2.5 transition-colors">Live</button>
                        <button className="text-gray-500 hover:text-gray-300 text-sm py-2.5 transition-colors">Output</button>
                        <button className="text-gray-500 hover:text-gray-300 text-sm py-2.5 transition-colors">Actions</button>
                    </div>

                    {/* Details Panel Content */}
                  <div className="flex-1 overflow-auto custom-scrollbar p-4">
                      <div className="space-y-1">
                          <TicketRow status="done" title="Implement user routing and inventory endpoints" cost="$1.89" tag="LANDED" />
                          <TicketRow status="done" title="Implement four-slot saved team creation and editing" cost="$7.20" tag="LANDED" />
                          <TicketRow status="running" title="Implement distinct PvE and PvP planning workflows" tag="RUNNING" />
                          <TicketRow status="queued" title="Implement Arena and RTA meta views with provenance" tag="QUEUED" />
                          <TicketRow status="queued" title="Implement safe, traceable, resilient game updates" tag="QUEUED" />
                          <TicketRow status="queued" title="Document runtime, fresh-data policy, persistence, and asset ..." tag="QUEUED" />
                      </div>

                      <div className="mt-8">
                          <h3 className="text-xs font-bold text-gray-500 mb-4 tracking-wider">ATTEMPT HISTORY</h3>
                          <table className="w-full text-sm text-left text-gray-400">
                              <thead className="text-xs text-gray-500 uppercase bg-[#0d1117] border-y border-[#30363d]">
                                  <tr>
                                      <th className="px-4 py-2">#</th>
                                      <th className="px-4 py-2">Status</th>
                                      <th className="px-4 py-2">Class</th>
                                      <th className="px-4 py-2">Cost</th>
                                      <th className="px-4 py-2">Duration</th>
                                  </tr>
                              </thead>
                              <tbody>
                                  <tr className="border-b border-[#30363d] bg-[#161b22]">
                                      <td className="px-4 py-3">1</td>
                                      <td className="px-4 py-3 text-teal-400">Running</td>
                                      <td className="px-4 py-3">—</td>
                                      <td className="px-4 py-3">—</td>
                                      <td className="px-4 py-3">—</td>
                                  </tr>
                              </tbody>
                          </table>
                      </div>
                  </div>
              </div>
          </div>
        </div>
      </div>
      )}
      
      {/* Global Styles for Scrollbar */}
      <style dangerouslySetInnerHTML={{__html: `
        .custom-scrollbar::-webkit-scrollbar {
          width: 8px;
          height: 8px;
        }
        .custom-scrollbar::-webkit-scrollbar-track {
          background: transparent; 
        }
        .custom-scrollbar::-webkit-scrollbar-thumb {
          background: #30363d; 
          border-radius: 4px;
        }
        .custom-scrollbar::-webkit-scrollbar-thumb:hover {
          background: #484f58; 
        }
      `}} />
    </div>
  );
};

const SidebarIcon = ({ icon, active }) => (
  <div className={`p-2 rounded-lg cursor-pointer transition-colors ${active ? 'bg-[#30363d] text-white' : 'text-gray-500 hover:bg-[#1c2128] hover:text-gray-300'}`}>
    {React.cloneElement(icon, { size: 20 })}
  </div>
);

const MetricItem = ({ label, value, color = "text-white" }) => (
  <div className="flex flex-col justify-center">
    <span className="text-[10px] text-gray-500 font-bold tracking-wider leading-none mb-1">{label}</span>
    <span className={`text-sm font-mono leading-none whitespace-nowrap ${color}`}>{value}</span>
  </div>
);

const GraphNode = ({ node, isLast, onClick }) => {
    
    const getStatusStyles = () => {
        if (node.status === 'RUNNING') return 'border-teal-500/50 bg-[#161b22] text-white shadow-[0_0_15px_rgba(20,184,166,0.15)] ring-1 ring-teal-500/20';
        if (node.status === 'COMPLETED') return 'border-gray-700 bg-[#1c2128] text-gray-300 opacity-80';
        return 'border-[#30363d] bg-[#0d1117] text-gray-500 border-dashed';
    };

    const getIcon = () => {
        if (node.status === 'RUNNING') return <CircleDashed size={14} className="text-teal-400 animate-spin-slow" />;
        if (node.status === 'COMPLETED') return <CheckCircle2 size={14} className="text-green-500" />;
        return <div className="w-3.5 h-3.5 rounded-full border border-gray-600" />;
    };

    return (
        <div className="relative group cursor-pointer" onClick={onClick}>
            <div className={`
                flex items-center gap-3 px-4 py-2.5 rounded-lg border w-64
                transition-all duration-200 hover:border-gray-500 z-10 relative
                ${getStatusStyles()}
            `}>
                <div className="shrink-0">
                    {getIcon()}
                </div>
                <div className="flex flex-col truncate">
                    <span className="text-xs font-medium truncate">{node.title}</span>
                </div>
            </div>
            
            {/* Connecting Line */}
            {!isLast && (
                <div className="absolute top-full left-1/2 -ml-px w-[2px] h-8 bg-gradient-to-b from-gray-700 to-transparent pointer-events-none" />
            )}
            {!isLast && node.status === 'RUNNING' && (
                <div className="absolute top-full left-1/2 -ml-px w-[2px] h-8 bg-gradient-to-b from-teal-500/50 to-transparent pointer-events-none" />
            )}
        </div>
    );
};

const TicketRow = ({ status, title, cost, tag }) => {
    const isDone = status === 'done';
    const isRunning = status === 'running';
    
    return (
        <div className={`flex items-center justify-between p-2 rounded-md hover:bg-[#1c2128] transition-colors border border-transparent ${isRunning ? 'bg-[#1c2128]/50 border-[#30363d]' : ''}`}>
            <div className="flex items-center gap-3 overflow-hidden">
                <div className="shrink-0">
                    {isDone && <Check size={14} className="text-green-500" />}
                    {isRunning && <CircleDashed size={14} className="text-teal-400 animate-spin" />}
                    {!isDone && !isRunning && <div className="w-3.5 h-3.5 rounded-full border border-gray-700" />}
                </div>
                <span className={`text-sm truncate ${isDone ? 'text-gray-400 line-through' : isRunning ? 'text-white' : 'text-gray-500'}`}>
                    {title}
                </span>
            </div>
            
            <div className="flex items-center gap-3 shrink-0 ml-4">
                {cost && <span className="text-xs text-gray-400 font-mono">{cost}</span>}
                {tag && (
                    <span className={`text-[10px] font-bold px-1.5 py-0.5 rounded ${
                        tag === 'LANDED' ? 'text-green-500 bg-green-500/10' :
                        tag === 'RUNNING' ? 'text-teal-400 bg-teal-400/10' :
                        'text-gray-500 bg-gray-800'
                    }`}>
                        {tag}
                    </span>
                )}
            </div>
        </div>
    );
};

const TimelineView = () => {
    const [isActivityOpen, setIsActivityOpen] = useState(true);

    const logs = [
        { time: '11:10:29 PM', action: 'Submitted', detail: 'Multiple instances of demeteo' },
        { time: '11:10:30 PM', action: 'Project bootstrapped', detail: 'p1784347830008' },
        { time: '11:10:30 PM', action: 'bootstrap_progress', detail: '{"detail":null,"label":"Cloning repository","phase":"cloning","status":"running"}' },
        { time: '11:10:31 PM', action: 'bootstrap_progress', detail: '{"detail":null,"label":"Cloning repository","phase":"cloning","status":"completed"}' },
        { time: '11:10:31 PM', action: 'bootstrap_progress', detail: '{"detail":null,"label":"Detecting project layout","phase":"detecting_strategy","status":"running"}' },
        { time: '11:10:31 PM', action: 'Repository cloned', detail: 'stevenyepes/demeteo' },
        { time: '11:10:31 PM', action: 'Feature started', detail: 'f-29779f5aa2cfd247' },
        { time: '11:10:31 PM', action: 'bootstrap_progress', detail: '{"detail":null,"label":"Loading project & workflow","phase":"preparing","status":"running"}' }
    ];

    const timelineSteps = [
        {
            id: 1,
            title: 'Research',
            type: 'agent',
            cost: '$0.959',
            tokens: '12.7k',
            duration: '3m 23s',
            outputs: [{ name: 'research-report.md', type: 'MARKDOWN' }]
        },
        {
            id: 2,
            title: 'Tickets',
            type: 'agent',
            cost: '$0.443',
            tokens: '12.4k',
            duration: '1m 44s',
            replay: true,
            subtext: '1 file changed • use Browse Code to review'
        },
        {
            id: 3,
            title: 'Spec',
            type: 'agent',
            cost: '$0.350',
            tokens: '10.8k',
            duration: '1m 18s',
            outputs: [{ name: 'implementation-spec.md', type: 'MARKDOWN' }]
        },
        {
            id: 4,
            title: 'Gate Review',
            type: 'gate',
            cost: '0',
            tokens: '0',
            duration: '1s',
            outputs: [{ name: 'implementation-spec.md', type: 'MARKDOWN' }]
        }
    ];

    return (
        <div className="flex-1 flex flex-col bg-[#090c10] overflow-hidden relative">
            
            {/* PR Context Bar */}
            <div className="flex items-center justify-between px-6 py-2.5 border-b border-[#30363d] bg-[#161b22] shrink-0 text-sm">
                <div className="flex items-center gap-3">
                    <GitPullRequest size={16} className="text-teal-400" />
                    <span className="text-teal-400 border border-teal-400/30 bg-teal-400/10 px-1.5 py-0.5 rounded text-xs font-bold uppercase tracking-wider">Open</span>
                    <a href="#" className="text-gray-400 hover:text-white transition-colors underline-offset-4 hover:underline">https://github.com/stevenyepes/demeteo/pull/89</a>
                </div>
                <button className="text-[10px] font-bold tracking-wider text-gray-500 hover:text-gray-300 transition-colors uppercase">
                    Refresh
                </button>
            </div>

            <div className="flex-1 overflow-auto custom-scrollbar p-6">
                
                {/* Expandable Activity Block */}
                <div className="border border-[#30363d] rounded-lg overflow-hidden bg-[#161b22] max-w-5xl mx-auto shadow-sm">
                    <div 
                        className="flex items-center justify-between p-3 border-b border-[#30363d] cursor-pointer hover:bg-[#1c2128] transition-colors"
                        onClick={() => setIsActivityOpen(!isActivityOpen)}
                    >
                        <div className="flex items-center gap-3">
                            <Activity size={16} className="text-teal-400" />
                            <span className="text-sm font-bold tracking-wider text-white">ACTIVITY</span>
                            <span className="text-xs text-gray-500 ml-2 font-mono">demeteo-remote • run laptop-321b207adfcd1245</span>
                        </div>
                        {isActivityOpen ? <ChevronUp size={16} className="text-gray-500" /> : <ChevronDown size={16} className="text-gray-500" />}
                    </div>
                    
                    {isActivityOpen && (
                        <div className="p-4 bg-[#090c10] font-mono text-[11px] leading-[1.8] text-gray-400 h-56 overflow-auto custom-scrollbar">
                            {logs.map((log, i) => (
                                <div key={i} className="flex hover:bg-[#161b22] px-2 py-0.5 -mx-2 rounded transition-colors">
                                    <span className="text-gray-600 w-28 shrink-0">{log.time}</span>
                                    <span className="text-teal-400/80 w-44 shrink-0">{log.action}</span>
                                    <span className="text-gray-400 truncate opacity-80">{log.detail}</span>
                                </div>
                            ))}
                            <div className="mt-4 text-gray-600">Last synced just now • polling every 3s</div>
                        </div>
                    )}
                </div>

                {/* Timeline List */}
                <div className="relative mt-8 max-w-5xl mx-auto w-full pb-10">
                    {/* Master Vertical Line */}
                    <div className="absolute left-[15px] top-4 bottom-0 w-[2px] bg-[#30363d]" />
                    
                    {timelineSteps.map((step) => (
                        <div key={step.id} className="relative pl-12 mb-6">
                            {/* Number Circle */}
                            <div className="absolute left-0 top-3 w-8 h-8 rounded-full bg-[#0d1117] border border-[#30363d] flex items-center justify-center text-xs font-bold text-gray-500 z-10 shadow-sm">
                                {step.id}
                            </div>
                            
                            {/* Card Container */}
                            <div className="bg-[#0d1117] border border-[#30363d] rounded-lg p-5 hover:border-[#484f58] transition-colors shadow-sm relative overflow-hidden group">
                                
                                <div className="flex flex-col sm:flex-row sm:items-start justify-between mb-4 gap-4">
                                    {/* Card Header Left */}
                                    <div className="flex items-center gap-3">
                                        <CheckCircle2 size={18} className="text-green-500 shrink-0" />
                                        <h3 className="text-base font-bold text-white tracking-wide">{step.title}</h3>
                                        <span className="text-[10px] font-bold tracking-wider uppercase bg-[#161b22] border border-[#30363d] text-gray-400 px-2 py-0.5 rounded">
                                            {step.type}
                                        </span>
                                        {step.replay && (
                                            <button className="flex items-center gap-1 text-[10px] font-bold text-teal-400 hover:text-teal-300 uppercase tracking-wider ml-2 transition-colors">
                                                <RotateCcw size={12} /> Replay
                                            </button>
                                        )}
                                    </div>
                                    
                                    {/* Card Header Right - Metrics */}
                                    <div className="flex items-center gap-5 text-xs font-mono shrink-0">
                                        <span className="text-green-400">{step.cost}</span>
                                        <span className="text-teal-400">{step.tokens}</span>
                                        <span className="text-gray-500">{step.duration}</span>
                                    </div>
                                </div>
                                
                                {/* Card Body / Outputs */}
                                {step.outputs && step.outputs.map((out, i) => (
                                    <div key={i} className="p-3 bg-[#161b22] rounded border border-[#30363d] flex items-center justify-between group-hover:border-[#484f58] transition-colors cursor-pointer">
                                        <div className="flex items-center gap-3">
                                            <FileText size={14} className="text-teal-400" />
                                            <span className="text-sm text-gray-300 font-medium">{out.name}</span>
                                        </div>
                                        <span className="text-[9px] font-bold tracking-wider text-gray-500 bg-[#0d1117] px-2 py-1 rounded">
                                            {out.type}
                                        </span>
                                    </div>
                                ))}

                                {/* Optional Subtext */}
                                {step.subtext && (
                                    <div className="text-xs text-gray-500 flex items-center gap-2 mt-2 ml-1">
                                        <span>1 file changed</span>
                                        <span className="text-gray-700">•</span>
                                        <span>use <strong className="text-gray-400 font-medium cursor-pointer hover:text-gray-300 transition-colors">Browse Code</strong> to review</span>
                                    </div>
                                )}
                            </div>
                        </div>
                    ))}
                </div>

            </div>
        </div>
    );
};

const ProjectDashboard = ({ onOpenPipeline }) => {
    const pipelines = [
        {
            id: 1,
            title: "Multiple instances of demeteo",
            desc: "Multiple instances of demeteo, when using the app launcher for opening demeteo its creating new instances instead of opening the existing one",
            status: "COMPLETED",
            statusColor: "text-green-500 border-green-500/30 bg-green-500/10",
            leftBorder: "border-green-500",
            workflow: "workflow: Standard Feature",
            remote: "DETACHED",
            branch: "f-29779f5aa2cfd247",
            duration: "31850s",
            tokens: "81.2k"
        },
        {
            id: 2,
            title: "duplicate notifications for merged PRs",
            desc: "duplicate notifications for merged PRs, weird thing. its happening with just that specific feature",
            status: "FAILED",
            statusColor: "text-red-400 border-red-400/30 bg-red-400/10",
            leftBorder: "border-red-500",
            workflow: "workflow: Capfix Pipeline",
            remote: "DETACHED",
            branch: "f-d9733cc39cefe3f7d",
            duration: "0s",
            tokens: "84.8k"
        },
        {
            id: 3,
            title: "Minimize the terminal",
            desc: "The current terminal for remote and local machines needs a better UX, currently if I try to navigate to another view, the terminal just closes and kill the running processes. I want to have a similar UX like the terminal in VSCODE, where you can add multiple terminals and also they are not killed while browsing the code or looking at the settings",
            status: "COMPLETED",
            statusColor: "text-green-500 border-green-500/30 bg-green-500/10",
            leftBorder: "border-green-500",
            workflow: "workflow: Standard Feature",
            remote: "DETACHED",
            branch: "f-6d0ee22942ee2fc0",
            duration: "52s",
            tokens: "568.5k"
        },
        {
            id: 4,
            title: "epic-builder",
            desc: "This is a simple web application, react with vite, its a modern nextjs app to create teams, keep an inventory of heroes, plan for pve and pvp, show the current meta for Arena and RTA. A space for updates to the game Epic Seven",
            status: "RUNNING",
            statusColor: "text-teal-400 border-teal-400/30 bg-teal-400/10",
            leftBorder: "border-teal-400",
            workflow: "workflow: Standard Feature",
            remote: "LOCAL",
            branch: "f-178684f890988",
            duration: "10m 56s",
            tokens: "600.7k"
        }
    ];

    return (
        <div className="flex-1 flex flex-col bg-[#090c10] overflow-hidden relative">
            <div className="flex-1 overflow-auto custom-scrollbar">
                <div className="max-w-6xl mx-auto w-full p-8 md:p-12">
                    
                    {/* Header */}
                    <div className="flex items-start justify-between mb-8">
                        <div>
                            <h1 className="text-3xl font-bold text-white mb-2 flex items-center gap-3">
                                <Server size={28} className="text-gray-500" />
                                demeteo-remote
                            </h1>
                            <p className="text-sm text-gray-400 flex items-center gap-2">
                                Connected via GitHub Enterprise • Default Workflow: Standard Feature Pipeline
                            </p>
                        </div>
                        <div className="flex gap-6 bg-[#161b22] px-5 py-3 rounded-xl border border-[#30363d]">
                            <div className="flex flex-col">
                                <span className="text-[10px] text-gray-500 font-bold tracking-wider uppercase mb-1">Fleet Active</span>
                                <span className="text-sm font-mono text-teal-400">0 Nodes</span>
                            </div>
                            <div className="w-px bg-[#30363d]"></div>
                            <div className="flex flex-col">
                                <span className="text-[10px] text-gray-500 font-bold tracking-wider uppercase mb-1">Total Spent</span>
                                <span className="text-sm font-mono text-gray-300">350.1k</span>
                            </div>
                        </div>
                    </div>

                    {/* Controls Row */}
                    <div className="flex items-center gap-3 mb-8">
                        <div className="flex items-center gap-2 bg-[#161b22] border border-[#30363d] px-3 py-2 rounded-lg text-sm text-gray-300 cursor-pointer hover:bg-[#1c2128] transition-colors">
                            <Server size={14} className="text-gray-500" />
                            <span>Auto - Checkout</span>
                            <ChevronDown size={14} className="text-gray-500 ml-4" />
                        </div>
                        <button className="flex items-center gap-2 bg-purple-600 hover:bg-purple-500 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors shadow-sm">
                            <Play size={14} fill="currentColor" />
                            START SESSION
                            <ChevronDown size={14} className="opacity-70 ml-1" />
                        </button>
                    </div>

                    {/* Tabs */}
                    <div className="flex border-b border-[#30363d] mb-6 gap-6">
                        <button className="text-white text-sm font-medium border-b-2 border-teal-400 py-3 flex items-center gap-2">
                            <GitPullRequest size={16} className="text-teal-400" />
                            Pipelines
                        </button>
                        <button className="text-gray-500 hover:text-gray-300 text-sm py-3 transition-colors flex items-center gap-2">
                            <TerminalSquare size={16} />
                            Terminal
                        </button>
                    </div>

                    {/* Prompt Box */}
                    <div className="bg-[#161b22] border border-[#30363d] rounded-xl p-4 mb-10 shadow-sm focus-within:border-gray-500 transition-colors">
                        <div className="flex items-start gap-3">
                            <Sparkles size={18} className="text-purple-400 mt-1 shrink-0" />
                            <div className="flex-1">
                                <textarea 
                                    className="w-full bg-transparent border-none text-gray-300 text-base resize-none focus:outline-none placeholder:text-gray-600 min-h-[60px]"
                                    placeholder="Draft and delegate a new feature pipeline..."
                                ></textarea>
                                <div className="flex items-center justify-between mt-2 pt-3 border-t border-[#30363d]/50">
                                    <div className="flex items-center gap-4 text-xs text-gray-500">
                                        <span>Press <kbd className="bg-[#0d1117] border border-[#30363d] rounded px-1.5 py-0.5 text-gray-400 mx-1 font-sans">Enter</kbd> to configure a launch</span>
                                        <span>•</span>
                                        <span>Paste an image to attach</span>
                                    </div>
                                    <button className="bg-purple-600/20 hover:bg-purple-600/30 text-purple-400 px-4 py-1.5 rounded text-sm font-medium transition-colors border border-purple-600/30 flex items-center gap-2">
                                        Continue <ChevronRight size={14} />
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>

                    {/* Pipelines List */}
                    <div>
                        <h2 className="text-xs font-bold text-gray-500 tracking-wider uppercase mb-4">Feature Pipelines</h2>
                        <div className="space-y-3">
                            {pipelines.map(pipe => (
                                <div 
                                    key={pipe.id}
                                    onClick={onOpenPipeline}
                                    className={`
                                        bg-[#161b22] border border-[#30363d] border-l-4 ${pipe.leftBorder} 
                                        rounded-lg p-4 cursor-pointer hover:bg-[#1c2128] hover:border-r-gray-500 hover:border-y-gray-500
                                        transition-all duration-200 group relative
                                    `}
                                >
                                    <div className="flex flex-col md:flex-row items-start md:items-center justify-between gap-4 mb-3">
                                        <div className="flex flex-wrap items-center gap-2 shrink-0">
                                            <span className={`px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider rounded border ${pipe.statusColor}`}>
                                                {pipe.status}
                                            </span>
                                            <span className="px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider rounded bg-purple-500/10 border border-purple-500/20 text-purple-400">
                                                {pipe.workflow}
                                            </span>
                                            <span className="px-2 py-0.5 text-[9px] font-bold uppercase tracking-wider rounded bg-blue-500/10 border border-blue-500/20 text-blue-400 flex items-center gap-1">
                                                <RotateCcw size={10} /> {pipe.remote}
                                            </span>
                                            <span className="text-xs text-gray-500 font-mono ml-1">{pipe.branch}</span>
                                        </div>
                                        
                                        <div className="flex items-center gap-6 text-xs font-mono shrink-0">
                                            <div className="flex flex-col md:items-end">
                                                <span className="text-[9px] text-gray-600 tracking-wider uppercase">Duration</span>
                                                <span className="text-gray-400">{pipe.duration}</span>
                                            </div>
                                            <div className="flex flex-col md:items-end">
                                                <span className="text-[9px] text-gray-600 tracking-wider uppercase">Tokens</span>
                                                <span className="text-teal-400">{pipe.tokens}</span>
                                            </div>
                                        </div>
                                    </div>
                                    
                                    <h3 className="text-base font-bold text-gray-200 mb-1.5 group-hover:text-white transition-colors">{pipe.title}</h3>
                                    <p className="text-sm text-gray-500 line-clamp-2 leading-relaxed">
                                        {pipe.desc}
                                    </p>
                                </div>
                            ))}
                        </div>
                    </div>

                </div>
            </div>
        </div>
    );
};

export default App;