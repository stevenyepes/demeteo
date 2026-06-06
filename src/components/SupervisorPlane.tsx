import React from "react";
import { CheckCircle2, ShieldAlert, ChevronRight, Eye, Check, Send, CircleDashed } from "lucide-react";
import { StreamEvent, ThreadSession } from "../types";

interface SupervisorPlaneProps {
  activeThreadId: string | null;
  threads: ThreadSession[];
  streams: Record<string, StreamEvent[]>;
  supervisorInput: string;
  setSupervisorInput: (val: string) => void;
  onSendDirective: (threadId: string) => void;
  onInspectContext: (path: string) => void;
  onApproveAction: (threadId: string, eventId: string) => void;
  onRejectAction: (threadId: string, eventId: string, feedback: string) => void;
}

const SupervisorPlane: React.FC<SupervisorPlaneProps> = ({
  activeThreadId,
  threads,
  streams,
  supervisorInput,
  setSupervisorInput,
  onSendDirective,
  onInspectContext,
  onApproveAction,
  onRejectAction,
}) => {
  const events = activeThreadId ? streams[activeThreadId] || [] : [];
  const isRunning = activeThreadId ? threads.find((t) => t.id === activeThreadId)?.status === "running" : false;

  const renderStream = () => {
    if (!activeThreadId) return null;
    const renderedElements: React.ReactNode[] = [];
    let currentGroup: StreamEvent[] = [];

    const flushGroup = (key: string) => {
      if (currentGroup.length === 0) return;
      const groupItems = [...currentGroup];
      currentGroup = [];

      renderedElements.push(
        <div key={key} className="pl-6 border-l-2 border-slate-800 flex flex-col gap-3 py-2 animate-slide-in">
          {groupItems.map((ev) => {
            if (ev.type === "auto_approve") {
              const cmd = ev.message.replace("Agent executed ", "");
              return (
                <div key={ev.id} className="flex items-center text-slate-500 text-xs py-0.5">
                  <CheckCircle2 size={12} className="text-slate-600 mr-1.5 flex-shrink-0" />
                  <span className="text-slate-600 mr-1.5 font-medium">[Auto-Approved]</span>
                  <span className="mr-1.5">Agent executed</span>
                  <code className="bg-white/5 px-1.5 py-0.5 rounded text-slate-400 font-mono">{cmd}</code>
                </div>
              );
            }
            return (
              <div key={ev.id} className="text-slate-500 text-xs flex items-center py-0.5">
                <CheckCircle2 size={12} className="text-slate-600 mr-1.5 flex-shrink-0" />
                <span>{ev.message}</span>
              </div>
            );
          })}
        </div>
      );
    };

    events.forEach((ev, idx) => {
      if (ev.type === "directive") {
        flushGroup(`group_before_dir_${idx}`);
        renderedElements.push(
          <div key={ev.id} className="flex items-start text-slate-300 gap-4 animate-slide-in bg-[#12161e]/75 p-4 rounded-xl border border-white/5 shadow-md">
            <span className="text-cyan-400 mt-0.5 font-bold font-mono text-base">&gt;</span>
            <div className="leading-relaxed text-sm">{ev.message}</div>
          </div>
        );
      } else if (ev.type === "intercept") {
        flushGroup(`group_before_int_${idx}`);
        
        // Clean trailing newlines to prevent empty line rendering bugs in split
        const rawCode = ev.payload?.code || "";
        const codeLines = rawCode.replace(/\n$/, "").split("\n");

        renderedElements.push(
          <div key={ev.id} className="bg-[#0a0a0e]/95 border border-amber-500/30 rounded-xl overflow-hidden shadow-xl shadow-amber-500/5 hover:border-amber-500/50 transition-all duration-300 animate-slide-in">
            <div className="px-4 py-2.5 bg-amber-500/10 border-b border-amber-500/20 flex items-center justify-between">
              <div className="flex items-center text-xs font-semibold text-amber-500 tracking-wide uppercase gap-2">
                <ShieldAlert size={14} className="mr-1" />
                <span>Intercepted Action: File Write</span>
              </div>
              <div className="text-[10px] text-slate-500">Agent: OpenCode</div>
            </div>
            <div className="p-5">
              <div className="text-xs text-slate-400 mb-3 flex items-center">
                <ChevronRight size={14} className="text-slate-600 mr-1" />
                <span>Target:</span>
                <span className="font-mono text-cyan-400 ml-1.5">{ev.payload?.path}</span>
              </div>
              <div className="bg-[#050508] border border-white/5 rounded-lg p-5 font-mono text-[13px] leading-relaxed relative max-h-60 overflow-y-auto whitespace-pre overflow-x-auto shadow-inner">
                <div className="absolute top-2 right-2 text-[10px] bg-emerald-500/20 text-emerald-400 px-1.5 rounded">+ {ev.payload?.additions} lines</div>
                {codeLines.map((line, lIdx) => {
                  const hasPlus = line.startsWith("+");
                  return (
                    <div key={lIdx} className="text-emerald-400">
                      {hasPlus ? line : `+ ${line}`}
                    </div>
                  );
                })}
              </div>
            </div>
            <div className="px-4 py-3 bg-slate-950/80 flex items-center justify-between border-t border-white/5">
              <button
                type="button"
                onClick={() => ev.payload?.path && onInspectContext(ev.payload.path)}
                className="px-3 py-1.5 rounded-lg border border-white/10 text-xs text-slate-300 hover:bg-white/5 transition-colors flex items-center gap-1.5"
              >
                <Eye size={14} className="mr-1" />
                <span>Inspect Context</span>
              </button>
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={() => {
                    const reason = window.prompt("Enter rejection feedback:");
                    if (reason !== null) onRejectAction(activeThreadId, ev.id, reason);
                  }}
                  className="px-4 py-1.5 rounded-lg border border-red-500/30 bg-red-500/10 text-red-400 text-xs hover:bg-red-500/20 transition-colors font-medium"
                >
                  Reject
                </button>
                <button
                  type="button"
                  onClick={() => onApproveAction(activeThreadId, ev.id)}
                  className="px-4 py-1.5 rounded-lg bg-cyan-500 text-slate-950 text-xs hover:bg-cyan-400 transition-colors shadow-[0_0_15px_rgba(6,182,212,0.4)] font-bold flex items-center gap-1.5"
                >
                  <Check size={14} className="mr-1" />
                  <span>Approve Execution</span>
                </button>
              </div>
            </div>
          </div>
        );
      } else {
        currentGroup.push(ev);
      }
    });

    if (currentGroup.length > 0 || isRunning) {
      const groupItems = [...currentGroup];
      renderedElements.push(
        <div key="final_group" className="pl-6 border-l-2 border-slate-800 flex flex-col gap-3 py-2 animate-slide-in">
          {groupItems.map((ev) => {
            if (ev.type === "auto_approve") {
              const cmd = ev.message.replace("Agent executed ", "");
              return (
                <div key={ev.id} className="flex items-center text-slate-500 text-xs py-0.5">
                  <CheckCircle2 size={12} className="text-slate-600 mr-1.5 flex-shrink-0" />
                  <span className="text-slate-600 mr-1.5 font-medium">[Auto-Approved]</span>
                  <span className="mr-1.5">Agent executed</span>
                  <code className="bg-white/5 px-1.5 py-0.5 rounded text-slate-400 font-mono">{cmd}</code>
                </div>
              );
            }
            return (
              <div key={ev.id} className="text-slate-500 text-xs flex items-center py-0.5">
                <CheckCircle2 size={12} className="text-slate-600 mr-1.5 flex-shrink-0" />
                <span>{ev.message}</span>
              </div>
            );
          })}
          {isRunning && (
            <div className="flex items-center text-cyan-500/70 text-xs py-0.5">
              <CircleDashed size={12} className="animate-spin-slow mr-1.5" />
              <span>Agent synthesizing response...</span>
            </div>
          )}
        </div>
      );
    }

    return renderedElements;
  };

  return (
    <>
      {/* Event Stream */}
      <div className="supervisor-stream-container flex-1">
        <div className="supervisor-stream-content">
          {renderStream()}

          {(!activeThreadId || (streams[activeThreadId] || []).length === 0) && (
            <div className="flex flex-col justify-center items-center py-20 text-slate-500 select-none">
              <CheckCircle2 size={32} className="mb-2 text-slate-600" />
              <div>Select or launch a thread to inspect logs.</div>
            </div>
          )}
        </div>
      </div>

      {/* Supervisor Input Box */}
      <div className="p-4 bg-slate-950/80 border-t border-white/5 select-none">
        <div className="supervisor-input-wrapper">
          <span className="absolute left-4 text-cyan-500 font-bold z-10">{">"}</span>
          <input
            type="text"
            placeholder="Enter directive or feedback for the agent..."
            className="supervisor-input-field bg-[#0a0a0e] border border-white/10 rounded-xl py-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50 focus:shadow-[0_0_15px_rgba(6,182,212,0.15)] transition-all font-mono placeholder-slate-600"
            value={supervisorInput}
            onChange={(e) => setSupervisorInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && activeThreadId && onSendDirective(activeThreadId)}
            disabled={!activeThreadId}
          />
          <button
            type="button"
            onClick={() => activeThreadId && onSendDirective(activeThreadId)}
            className="absolute right-2 p-2 rounded-lg bg-cyan-500/10 text-cyan-400 hover:bg-cyan-500 hover:text-slate-900 transition-all flex items-center justify-center z-10"
          >
            <Send size={16} />
          </button>
        </div>
        <div className="text-center mt-2 text-[10px] text-slate-600 font-mono">
          Demeteo PDP Active • OpenCode Container Running
        </div>
      </div>
    </>
  );
};

export default SupervisorPlane;
