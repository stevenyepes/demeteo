import React from "react";
import {
  TerminalSquare,
  Server,
  Activity,
  Plus,
  Edit2,
  Trash2,
  Check,
  Bot,
  GitBranch,
  Terminal,
  MemoryStick,
  FileCode2,
} from "lucide-react";
import { FrontendMachine, ThreadSession, FileReference } from "../types";

interface SidebarProps {
  isCollapsed: boolean;
  machinesList: FrontendMachine[];
  activeMachine: FrontendMachine | null;
  showMachineSelector: boolean;
  setShowMachineSelector: (show: boolean) => void;
  onMachineSelect: (m: FrontendMachine) => void;
  onAddEnv: () => void;
  onEditEnv: (m: FrontendMachine, e: React.MouseEvent) => void;
  onDeleteEnv: (id: string, e: React.MouseEvent) => void;
  threads: ThreadSession[];
  activeThreadId: string | null;
  onThreadSelect: (id: string) => void;
  setWorkspaceMode: (mode: string) => void;
  onNewThreadClick: () => void;
  onDeleteThread: (id: string) => void;
  workingMemory: FileReference[];
  inspectedFileName: string | undefined;
  onInspectFile: (path: string) => void;
}

const Sidebar: React.FC<SidebarProps> = ({
  isCollapsed,
  machinesList,
  activeMachine,
  showMachineSelector,
  setShowMachineSelector,
  onMachineSelect,
  onAddEnv,
  onEditEnv,
  onDeleteEnv,
  threads,
  activeThreadId,
  onThreadSelect,
  setWorkspaceMode,
  onNewThreadClick,
  onDeleteThread,
  workingMemory,
  inspectedFileName,
  onInspectFile,
}) => {
  return (
    <div className={`flex-shrink-0 bg-[#0a0a0e] border-r border-white/5 flex flex-col z-30 shadow-xl h-full select-none transition-all duration-300 ${isCollapsed ? 'w-0 overflow-hidden border-r-0' : 'w-80'}`}>
      
      {/* Environment Selector Dropdown */}
      <div className="p-4 border-b border-white/5 relative">
        <div className="text-[10px] uppercase tracking-wider text-slate-500 font-semibold mb-2 flex justify-between items-center">
          Target Environment
          <button type="button" onClick={onAddEnv} className="hover:text-cyan-400 transition-colors" title="Add Environment">
            <Plus size={12} />
          </button>
        </div>
        <div className="relative">
          {activeMachine ? (
            <button
              type="button"
              onClick={() => setShowMachineSelector(!showMachineSelector)}
              className="w-full bg-slate-900/80 border border-white/10 hover:border-white/20 rounded-xl p-3 flex items-center justify-between transition-all"
            >
              <div className="flex items-center">
                {activeMachine.type === "local" ? (
                  <TerminalSquare size={16} className="text-emerald-400 mr-3" />
                ) : (
                  <Server size={16} className="text-cyan-400 mr-3" />
                )}
                <div className="text-left">
                  <div className="text-sm font-medium text-slate-200">{activeMachine.name}</div>
                  <div className="text-[10px] text-slate-500 font-mono mt-0.5">{activeMachine.user}</div>
                </div>
              </div>
              <Activity size={14} className={activeMachine.status === "connected" ? "text-emerald-500" : "text-slate-600"} />
            </button>
          ) : (
            <button
              type="button"
              onClick={onAddEnv}
              className="w-full bg-slate-900/80 border border-white/10 rounded-xl p-3 text-center text-xs text-slate-400 hover:text-white"
            >
              + Register Dev Node
            </button>
          )}

          {/* Dropdown Menu */}
          {showMachineSelector && (
            <div className="absolute top-full left-0 w-full mt-2 bg-slate-900 border border-white/10 rounded-xl shadow-2xl z-50 overflow-hidden">
              <div className="max-h-64 overflow-y-auto">
                {machinesList.map((m) => (
                  <div
                    key={m.id}
                    onClick={() => onMachineSelect(m)}
                    className="p-3 hover:bg-white/5 cursor-pointer border-b border-white/5 last:border-0 flex items-center justify-between group"
                  >
                    <div className="flex items-center">
                      {m.type === "local" ? (
                        <TerminalSquare size={14} className="text-slate-500 mr-2" />
                      ) : (
                        <Server size={14} className="text-slate-500 mr-2" />
                      )}
                      <div className="text-left">
                        <div className="text-sm text-slate-200 font-medium">{m.name}</div>
                        <div className="text-[10px] text-slate-500 font-mono">{m.user}</div>
                      </div>
                    </div>
                    <div className="flex items-center gap-1">
                      <button
                        type="button"
                        onClick={(e) => onEditEnv(m, e)}
                        className="p-1.5 rounded-md text-slate-500 hover:text-cyan-400 hover:bg-cyan-500/10 transition-all opacity-0 group-hover:opacity-100"
                      >
                        <Edit2 size={14} />
                      </button>
                      <button
                        type="button"
                        onClick={(e) => onDeleteEnv(m.id, e)}
                        className="p-1.5 rounded-md text-slate-500 hover:text-red-400 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                      >
                        <Trash2 size={14} />
                      </button>
                      {activeMachine && m.id === activeMachine.id && <Check size={14} className="text-emerald-500 ml-1" />}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Scrollable Context Areas */}
      <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-10">
        
        {/* Active Threads Block */}
        <section>
          <div className="flex items-center justify-between text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
            <div className="flex items-center">
              <Bot size={14} className="mr-2 text-slate-500" /> Active Threads
            </div>
            <button
              type="button"
              onClick={onNewThreadClick}
              disabled={!activeMachine}
              className="p-1 rounded hover:bg-white/10 hover:text-cyan-400 transition-colors disabled:opacity-50"
            >
              <Plus size={14} />
            </button>
          </div>
          <div className="flex flex-col gap-3">
            {threads.map((thread) => (
              <div
                key={thread.id}
                onClick={() => {
                  onThreadSelect(thread.id);
                  setWorkspaceMode("supervisor");
                }}
                className={`thread-card p-2.5 rounded-lg cursor-pointer transition-all border group ${
                  activeThreadId === thread.id
                    ? "bg-cyan-500/10 border-cyan-500/30 shadow-[0_0_15px_rgba(6,182,212,0.1)]"
                    : "bg-white/5 border-white/5 hover:bg-white/10"
                }`}
              >
                <div className="flex items-center justify-between mb-1">
                  <span className={`text-xs font-medium truncate flex-1 mr-1 ${activeThreadId === thread.id ? "text-cyan-400" : "text-slate-300"}`}>
                    {thread.title}
                  </span>
                  <div className="flex items-center gap-1 flex-shrink-0">
                    {thread.status === "pending_approval" && <span className="w-2 h-2 rounded-full bg-amber-500 animate-pulse"></span>}
                    {thread.status === "running" && <span className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse"></span>}
                    <button
                      type="button"
                      onClick={(e) => { e.stopPropagation(); onDeleteThread(thread.id); }}
                      className="p-0.5 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                      title="Remove thread"
                    >
                      <Trash2 size={11} />
                    </button>
                  </div>
                </div>
                <div className="flex items-center text-[10px] text-slate-500 font-mono">
                  {thread.mode === "worktree" ? (
                    <>
                      <GitBranch size={10} className="mr-1" /> {thread.branch}
                    </>
                  ) : (
                    <>
                      <Terminal size={10} className="mr-1" /> Ad-Hoc Session
                    </>
                  )}
                </div>
              </div>
            ))}
            {threads.length === 0 && (
              <p style={{ color: "var(--text-muted)", fontSize: "0.8rem", textAlign: "center", padding: "10px" }}>
                No active sandboxes.
              </p>
            )}
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
          <div className="flex flex-col gap-3">
            {workingMemory.map((file, idx) => (
              <div
                key={idx}
                onClick={() => {
                  setWorkspaceMode("supervisor");
                  onInspectFile(file.name);
                }}
                className={`memory-card flex items-center justify-between p-2 rounded-md border cursor-pointer transition-all ${
                  inspectedFileName === file.name
                    ? "bg-white/10 border-white/20"
                    : "bg-white/5 border-white/5 hover:border-white/10"
                }`}
              >
                <div className="flex items-center text-xs text-slate-300">
                  <FileCode2
                    size={12}
                    className={`mr-2 ${inspectedFileName === file.name ? "text-cyan-400" : "text-slate-500"}`}
                  />
                  <span className={inspectedFileName === file.name ? "text-cyan-400 font-medium" : ""}>{file.name}</span>
                </div>
                <div className="flex items-center font-mono">
                  {file.isNew && <span className="text-[9px] text-emerald-400 mr-2 bg-emerald-500/10 px-1 rounded">NEW</span>}
                  <div className="text-[10px] text-slate-500">{file.lines}L</div>
                </div>
              </div>
            ))}
            {workingMemory.length === 0 && (
              <p style={{ color: "var(--text-muted)", fontSize: "0.8rem", textAlign: "center", padding: "10px" }}>
                Memory registers empty.
              </p>
            )}
          </div>
        </section>
      </div>
    </div>
  );
};

export default Sidebar;
