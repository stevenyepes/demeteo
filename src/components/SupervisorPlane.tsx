import React, { useState } from "react";
import { CheckCircle2, ShieldAlert, ChevronRight, Eye, Check, Send, CircleDashed, FlaskConical, Square } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { marked } from "marked";
import PromptDialog from "./PromptDialog";
import { AgentAction, CommandOutcome, StreamEvent, ThreadSession } from "../types";

// Configure marked: synchronous output, no mangle for clean HTML.
marked.setOptions({ async: false, gfm: true, breaks: true });

/** Render markdown to HTML string and strip the outer <p> wrapper for
 *  inline content so short responses don't get block-level padding. */
function renderMarkdown(text: string): string {
  const raw = marked.parse(text) as string;
  // If the result is a single paragraph, strip the wrapping <p>…</p>
  // so it flows naturally inside the flex container.
  const trimmed = raw.trim();
  if (trimmed.startsWith("<p>") && trimmed.endsWith("</p>") && trimmed.indexOf("<p>", 3) === -1) {
    return trimmed.slice(3, -4);
  }
  return raw;
}

/** Human-readable label for an intercept action kind. */
function actionLabel(action?: string): string {
  switch (action) {
    case "read": return "File Read";
    case "edit": return "File Edit";
    case "write": return "File Write";
    case "run_bash": return "Bash Command";
    default: return "Action";
  }
}

interface SupervisorPlaneProps {
  activeThreadId: string | null;
  threads: ThreadSession[];
  streams: Record<string, StreamEvent[]>;
  supervisorInput: string;
  setSupervisorInput: (val: string) => void;
  onSendDirective: (threadId: string) => void;
  onStopTurn: (threadId: string) => void;
  onInspectContext: (path: string) => void;
  onApproveAction: (threadId: string, eventId: string) => void;
  onRejectAction: (threadId: string, eventId: string, feedback: string) => void;
  activeMachineId: string | null;
}

const SupervisorPlane: React.FC<SupervisorPlaneProps> = ({
  activeThreadId,
  threads,
  streams,
  supervisorInput,
  setSupervisorInput,
  onSendDirective,
  onStopTurn,
  onInspectContext,
  onApproveAction,
  onRejectAction,
  activeMachineId,
}) => {
  const [testOpen, setTestOpen] = useState(false);
  const [testCmd, setTestCmd] = useState("cargo build");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string>("");
  const [rejectOpen, setRejectOpen] = useState(false);
  const [rejectEventId, setRejectEventId] = useState<string | null>(null);
  const events = activeThreadId ? streams[activeThreadId] || [] : [];
  const activeThread = activeThreadId ? threads.find((t) => t.id === activeThreadId) : null;
  const status = activeThread?.status ?? "idle";
  const isRunning = status === "running";
  const isPendingApproval = status === "pending_approval";

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
        
            const rawCode = ev.payload?.code || "";
        const codeLines = rawCode.replace(/\n$/, "").split("\n");
        const isBash = ev.payload?.action === "run_bash";
        const createdAt = ev.payload?.created_at
          ? new Date(Number(ev.payload.created_at.replace("Z", "")) * 1000).toLocaleTimeString()
          : ev.timestamp;
        const isAgentOriginated = !!ev.payload?.tool_call_id;

        renderedElements.push(
          <div key={ev.id} className="bg-[#0a0a0e]/95 border border-amber-500/30 rounded-xl overflow-hidden shadow-xl shadow-amber-500/5 hover:border-amber-500/50 transition-all duration-300 animate-slide-in">
            <div className="px-4 py-2.5 bg-amber-500/10 border-b border-amber-500/20 flex items-center justify-between">
              <div className="flex items-center text-xs font-semibold text-amber-500 tracking-wide uppercase gap-2">
                <ShieldAlert size={14} className="mr-1" />
                <span>Intercepted Action: {actionLabel(ev.payload?.action)}</span>
              </div>
              <div className="flex items-center gap-2 text-[10px] text-slate-500">
                {isAgentOriginated && <span>Agent Tool Call</span>}
                <span>{createdAt}</span>
              </div>
            </div>
            <div className="p-5">
              <div className="text-xs text-slate-400 mb-3 flex items-center">
                <ChevronRight size={14} className="text-slate-600 mr-1" />
                <span>Target:</span>
                <span className="font-mono text-cyan-400 ml-1.5 break-all">{ev.payload?.path}</span>
              </div>
              {rawCode && (
                <div className={`bg-[#050508] border border-white/5 rounded-lg p-5 font-mono text-[13px] leading-relaxed relative max-h-60 overflow-y-auto whitespace-pre overflow-x-auto shadow-inner ${isBash ? "" : ""}`}>
                  {isBash ? (
                    <code className="text-amber-300">{rawCode}</code>
                  ) : (
                    <>
                      <div className="absolute top-2 right-2 text-[10px] bg-emerald-500/20 text-emerald-400 px-1.5 rounded">{codeLines.length} lines</div>
                      {codeLines.map((line, lIdx) => (
                        <div key={lIdx} className="text-emerald-400">{line}</div>
                      ))}
                    </>
                  )}
                </div>
              )}
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
                    setRejectEventId(ev.id);
                    setRejectOpen(true);
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
      } else if (ev.type === "text") {
        flushGroup(`group_before_text_${idx}`);
        renderedElements.push(
          <div key={ev.id} className="flex flex-col text-slate-300 gap-2.5 animate-slide-in bg-[#12161e]/45 p-4 rounded-xl border border-cyan-500/10 backdrop-blur-[12px] shadow-[0_4px_20px_rgba(0,0,0,0.2)]">
            <div className="flex items-center gap-2 text-[10px] text-slate-400 font-semibold tracking-wider uppercase select-none">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)] animate-pulse"></span>
              <span>Agent</span>
            </div>
            <div
              className="leading-relaxed text-sm font-sans agent-markdown"
              dangerouslySetInnerHTML={{ __html: renderMarkdown(ev.message) }}
            />
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
            // Per AGENT_INTEGRATION §9.2: agent_error renders with the
            // ruby accent. We use the `agent_error` discriminator that
            // App.tsx emits when an AgentEvent::Error arrives.
            if (ev.type === "agent_error") {
              return (
                <div
                  key={ev.id}
                  className="flex items-center text-red-400 text-xs py-0.5 font-mono"
                >
                  <ShieldAlert size={12} className="text-red-500 mr-1.5 flex-shrink-0" />
                  <span className="text-red-500 mr-1.5 font-semibold">[Agent Error]</span>
                  <span>{ev.message}</span>
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
            placeholder={
              isRunning
                ? "Enter redirect — implicit cancel + send"
                : isPendingApproval
                ? "Resolve pending action first"
                : "Enter directive or feedback for the agent..."
            }
            className="supervisor-input-field bg-[#0a0a0e] border border-white/10 rounded-xl py-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50 focus:shadow-[0_0_15px_rgba(6,182,212,0.15)] transition-all font-mono placeholder-slate-600"
            value={supervisorInput}
            onChange={(e) => setSupervisorInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && activeThreadId && onSendDirective(activeThreadId)}
            disabled={!activeThreadId || isPendingApproval}
          />
          {/* Per AGENT_INTEGRATION §8.4: Stop replaces Send during
              `running`; the implicit cancel + redirect on Enter is
              handled in App.tsx's sendDirective (which calls
              agent_cancel then agent_prompt). */}
          {isRunning ? (
            <button
              type="button"
              onClick={() => activeThreadId && onStopTurn(activeThreadId)}
              title="Stop current turn"
              className="absolute right-2 p-2 rounded-lg bg-red-500/10 text-red-400 hover:bg-red-500 hover:text-slate-900 transition-all flex items-center justify-center z-10"
            >
              <Square size={14} fill="currentColor" />
            </button>
          ) : (
            <button
              type="button"
              onClick={() => activeThreadId && onSendDirective(activeThreadId)}
              disabled={!activeThreadId || isPendingApproval}
              className="absolute right-2 p-2 rounded-lg bg-cyan-500/10 text-cyan-400 hover:bg-cyan-500 hover:text-slate-900 transition-all flex items-center justify-center z-10 disabled:opacity-30 disabled:cursor-not-allowed"
            >
              <Send size={16} />
            </button>
          )}
        </div>

        {testOpen ? (
          <div className="mt-3 bg-[#0a0a0e] border border-amber-500/30 rounded-xl p-3 animate-in fade-in slide-in-from-bottom-2 duration-200">
            <div className="flex items-center justify-between mb-2">
              <div className="text-[10px] uppercase tracking-wider text-amber-500 font-semibold flex items-center gap-1.5">
                <FlaskConical size={11} />
                Test Intercept Pipeline
              </div>
              <button
                type="button"
                onClick={() => { setTestOpen(false); setTestResult(""); }}
                className="text-slate-500 hover:text-white text-xs"
              >
                Cancel
              </button>
            </div>
            <div className="flex gap-2">
              <input
                type="text"
                value={testCmd}
                onChange={(e) => setTestCmd(e.target.value)}
                placeholder="cargo build"
                className="flex-1 bg-[#050508] border border-white/10 rounded-lg py-1.5 px-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
                disabled={testing}
              />
              <button
                type="button"
                onClick={async () => {
                  if (!activeThreadId || !activeMachineId) return;
                  setTesting(true);
                  setTestResult("");
                  try {
                    const action: AgentAction = { RunBash: { cmd: testCmd } };
                    const outcome: CommandOutcome = await invoke("request_action", {
                      threadId: activeThreadId,
                      machineId: activeMachineId,
                      action,
                    });
                    if (outcome.kind === "executed") {
                      const r = outcome.output;
                      setTestResult(
                        r.kind === "bash"
                          ? `Executed immediately. Output: ${r.output || "(empty)"}`
                          : `Executed immediately: ${r.kind}`,
                      );
                    } else {
                      setTestResult(`Escalated to user. Intercept: ${outcome.intercept_id}`);
                    }
                  } catch (e) {
                    setTestResult(`Error: ${e}`);
                  } finally {
                    setTesting(false);
                  }
                }}
                disabled={testing || !activeThreadId || !activeMachineId}
                className="px-3 py-1.5 rounded-lg text-xs font-medium bg-amber-500/10 border border-amber-500/30 text-amber-400 hover:bg-amber-500/20 transition-colors disabled:opacity-50"
              >
                {testing ? "Submitting…" : "Submit"}
              </button>
            </div>
            {testResult && (
              <div className="mt-2 text-[11px] font-mono text-slate-300 break-all">
                {testResult}
              </div>
            )}
          </div>
        ) : (
          <div className="mt-2 text-center">
            <button
              type="button"
              onClick={() => setTestOpen(true)}
              disabled={!activeThreadId || !activeMachineId}
              className="text-[10px] text-slate-500 hover:text-amber-400 transition-colors disabled:opacity-50 flex items-center gap-1 mx-auto"
            >
              <FlaskConical size={10} />
              Test Intercept
            </button>
          </div>
        )}
        <div className="text-center mt-2 text-[10px] text-slate-600 font-mono">
          {activeThreadId
            ? (() => {
                const t = threads.find(t => t.id === activeThreadId);
                if (!t) return "No active thread";
                // Status map extended per AGENT_INTEGRATION §8.3.
                // The status field comes from the backend
                // (ThreadSession.status); we accept the union plus
                // legacy strings (the backend enforces, the UI
                // just maps to a friendly label).
                const statusMap: Record<string, string> = {
                  idle: "Thread Idle • Awaiting Directive",
                  running: "Agent Running • Supervisor Active",
                  pending_approval: "Pending Supervisor Approval",
                  spawning: "Spawning Agent…",
                  installing: "Installing Agent…",
                  error: "Agent Error • Action Required",
                };
                return statusMap[t.status] ?? `Status: ${t.status}`;
              })()
            : "Select a thread to begin"}
        </div>
      </div>

      <PromptDialog
        isOpen={rejectOpen && activeThreadId !== null}
        title="Reject Action"
        message="Enter rejection feedback sent to the agent:"
        placeholder="e.g. Avoid modifying configuration files"
        okLabel="Reject"
        danger
        onConfirm={(reason) => {
          if (activeThreadId && rejectEventId) {
            onRejectAction(activeThreadId, rejectEventId, reason);
          }
          setRejectOpen(false);
          setRejectEventId(null);
        }}
        onCancel={() => {
          setRejectOpen(false);
          setRejectEventId(null);
        }}
      />
    </>
  );
};

export default SupervisorPlane;
