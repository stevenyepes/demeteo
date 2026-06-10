import React, { useState, useRef, useEffect } from "react";
import { CheckCircle2, ShieldAlert, ChevronRight, Eye, Check, Send, CircleDashed, FlaskConical, Square } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { marked } from "marked";
import PromptDialog from "./PromptDialog";
import { AgentAction, CommandOutcome, Message, InterceptCard, ThreadSession } from "../types";

marked.setOptions({ async: false, gfm: true, breaks: true });

function renderMarkdown(text: string): string {
  const raw = marked.parse(text) as string;
  const trimmed = raw.trim();
  if (trimmed.startsWith("<p>") && trimmed.endsWith("</p>") && trimmed.indexOf("<p>", 3) === -1) {
    return trimmed.slice(3, -4);
  }
  return raw;
}

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
  messages: Record<string, Message[]>;
  intercepts: Record<string, InterceptCard[]>;
  pendingAssistantText: Record<string, string>;
  supervisorInput: string;
  setSupervisorInput: (val: string) => void;
  onSendDirective: (threadId: string) => void;
  onStopTurn: (threadId: string) => void;
  onInspectContext: (path: string) => void;
  onApproveAction: (threadId: string, cardId: string) => void;
  onRejectAction: (threadId: string, cardId: string, feedback: string) => void;
  activeMachineId: string | null;
}

const SupervisorPlane: React.FC<SupervisorPlaneProps> = ({
  activeThreadId,
  threads,
  messages,
  intercepts,
  pendingAssistantText,
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
  const [rejectCardId, setRejectCardId] = useState<string | null>(null);

  const threadMessages = activeThreadId ? messages[activeThreadId] || [] : [];
  const threadIntercepts = activeThreadId ? intercepts[activeThreadId] || [] : [];
  const pendingText = activeThreadId ? pendingAssistantText[activeThreadId] || "" : "";

  const scrollRef = useRef<HTMLDivElement>(null);
  const userScrolledUp = useRef(false);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || userScrolledUp.current) return;
    el.scrollTop = el.scrollHeight;
  }, [threadMessages.length, threadIntercepts.length, pendingText]);

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    userScrolledUp.current = !atBottom;
  };

  const activeThread = activeThreadId ? threads.find((t) => t.id === activeThreadId) : null;
  const status = activeThread?.status ?? "idle";
  const isRunning = status === "running";
  const isPendingApproval = status === "pending_approval";

  const renderContent = () => {
    if (!activeThreadId) return null;
    const elements: React.ReactNode[] = [];

    // Group system messages (info, errors) together
    let systemGroup: Message[] = [];

    const flushSystemGroup = (key: string) => {
      if (systemGroup.length === 0) return;
      const group = [...systemGroup];
      systemGroup = [];
      elements.push(
        <div key={key} className="stream-event pl-6 border-l-2 border-slate-800 flex flex-col gap-3 py-2 animate-slide-in">
          {group.map((m) => {
            const isError = m.metadata?.is_error;
            if (isError) {
              return (
                <div key={m.id} className="flex items-center text-red-400 text-xs py-0.5 font-mono">
                  <ShieldAlert size={12} className="text-red-500 mr-1.5 flex-shrink-0" />
                  <span className="text-red-500 mr-1.5 font-semibold">[Agent Error]</span>
                  <span>{m.content}</span>
                </div>
              );
            }
            if (m.metadata?.action === "bash_result") {
              const cmd = m.content;
              return (
                <div key={m.id} className="flex items-center text-slate-500 text-xs py-0.5">
                  <CheckCircle2 size={12} className="text-slate-600 mr-1.5 flex-shrink-0" />
                  <span className="text-slate-600 mr-1.5 font-medium">[Auto-Approved]</span>
                  <span className="mr-1.5">Agent executed</span>
                  <code className="bg-white/5 px-1.5 py-0.5 rounded text-slate-400 font-mono">{cmd}</code>
                </div>
              );
            }
            return (
              <div key={m.id} className="text-slate-500 text-xs flex items-center py-0.5">
                <CheckCircle2 size={12} className="text-slate-600 mr-1.5 flex-shrink-0" />
                <span>{m.content}</span>
              </div>
            );
          })}
        </div>
      );
    };

    let idxCounter = 0;

    // Render persisted messages
    for (const msg of threadMessages) {
      const idx = idxCounter++;
      if (msg.role === "user") {
        flushSystemGroup(`sys_before_user_${idx}`);
        elements.push(
          <div key={msg.id} className="stream-event flex items-start text-slate-300 gap-4 animate-slide-in bg-[#12161e]/75 p-4 rounded-xl border border-white/5 shadow-md">
            <span className="text-cyan-400 mt-0.5 font-bold font-mono text-base">&gt;</span>
            <div className="leading-relaxed text-sm">{msg.content}</div>
          </div>
        );
      } else if (msg.role === "assistant") {
        flushSystemGroup(`sys_before_asst_${idx}`);
        elements.push(
          <div key={msg.id} className="stream-event flex flex-col text-slate-300 gap-2.5 animate-slide-in bg-[#12161e]/45 p-4 rounded-xl border border-cyan-500/10 backdrop-blur-[12px] shadow-[0_4px_20px_rgba(0,0,0,0.2)]">
            <div className="flex items-center gap-2 text-[10px] text-slate-400 font-semibold tracking-wider uppercase select-none">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)]"></span>
              <span>Agent</span>
            </div>
            <div
              className="leading-relaxed text-sm font-sans agent-markdown"
              dangerouslySetInnerHTML={{ __html: renderMarkdown(msg.content) }}
            />
          </div>
        );
      } else {
        // system message
        systemGroup.push(msg);
      }
    }

    // Render pending streaming text
    if (pendingText) {
      flushSystemGroup(`sys_before_pending`);
      elements.push(
        <div key="pending" className="stream-event flex flex-col text-slate-300 gap-2.5 animate-slide-in bg-[#12161e]/45 p-4 rounded-xl border border-cyan-500/10 backdrop-blur-[12px] shadow-[0_4px_20px_rgba(0,0,0,0.2)]">
          <div className="flex items-center gap-2 text-[10px] text-slate-400 font-semibold tracking-wider uppercase select-none">
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)] animate-pulse"></span>
            <span>Agent</span>
          </div>
          <div
            className="leading-relaxed text-sm font-sans agent-markdown"
            dangerouslySetInnerHTML={{ __html: renderMarkdown(pendingText) }}
          />
        </div>
      );
    }

    // Render intercept cards (transient UI — not in messages)
    for (const card of threadIntercepts) {
      flushSystemGroup(`sys_before_int_${idxCounter++}`);
      const rawCode = card.code || "";
      const codeLines = rawCode.replace(/\n$/, "").split("\n");
      const isBash = card.action === "run_bash";
      const createdAt = card.created_at
        ? new Date(Number(card.created_at.replace("Z", "")) * 1000).toLocaleTimeString()
        : "";

      elements.push(
        <div key={card.id} className="stream-event bg-[#0a0a0e]/95 border border-amber-500/30 rounded-xl overflow-hidden shadow-xl shadow-amber-500/5 hover:border-amber-500/50 transition-all duration-300 animate-slide-in">
          <div className="px-4 py-2.5 bg-amber-500/10 border-b border-amber-500/20 flex items-center justify-between">
            <div className="flex items-center text-xs font-semibold text-amber-500 tracking-wide uppercase gap-2">
              <ShieldAlert size={14} className="mr-1" />
              <span>Intercepted Action: {actionLabel(card.action)}</span>
            </div>
            <div className="flex items-center gap-2 text-[10px] text-slate-500">
              {card.tool_call_id && <span>Agent Tool Call</span>}
              <span>{createdAt}</span>
            </div>
          </div>
          <div className="p-5">
            <div className="text-xs text-slate-400 mb-3 flex items-center">
              <ChevronRight size={14} className="text-slate-600 mr-1" />
              <span>Target:</span>
              <span className="font-mono text-cyan-400 ml-1.5 break-all">{card.target}</span>
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
          {card.status === 'pending' && (
            <div className="px-4 py-3 bg-slate-950/80 flex items-center justify-between border-t border-white/5">
              <button
                type="button"
                onClick={() => card.target && onInspectContext(card.target)}
                className="px-3 py-1.5 rounded-lg border border-white/10 text-xs text-slate-300 hover:bg-white/5 transition-colors flex items-center gap-1.5"
              >
                <Eye size={14} className="mr-1" />
                <span>Inspect Context</span>
              </button>
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={() => {
                    setRejectCardId(card.id);
                    setRejectOpen(true);
                  }}
                  className="px-4 py-1.5 rounded-lg border border-red-500/30 bg-red-500/10 text-red-400 text-xs hover:bg-red-500/20 transition-colors font-medium"
                >
                  Reject
                </button>
                <button
                  type="button"
                  onClick={() => onApproveAction(activeThreadId, card.id)}
                  className="px-4 py-1.5 rounded-lg bg-cyan-500 text-slate-950 text-xs hover:bg-cyan-400 transition-colors shadow-[0_0_15px_rgba(6,182,212,0.4)] font-bold flex items-center gap-1.5"
                >
                  <Check size={14} className="mr-1" />
                  <span>Approve Execution</span>
                </button>
              </div>
            </div>
          )}
          {card.status === 'approved' && (
            <div className="px-4 py-3 bg-slate-950/80 border-t border-white/5">
              <div className="text-xs text-emerald-400 font-medium">Approved by Supervisor</div>
            </div>
          )}
          {card.status === 'rejected' && (
            <div className="px-4 py-3 bg-slate-950/80 border-t border-white/5">
              <div className="text-xs text-red-400 font-medium">
                Rejected{card.feedback ? `: ${card.feedback}` : ""}
              </div>
            </div>
          )}
        </div>
      );
    }

    // Flush remaining system group
    const remainingKey = `sys_final_${idxCounter++}`;
    flushSystemGroup(remainingKey);

    // Running indicator
    if (isRunning) {
      elements.push(
        <div key="running_indicator" className="stream-event pl-6 border-l-2 border-slate-800 flex flex-col gap-3 py-2 animate-slide-in">
          <div className="flex items-center text-cyan-500/70 text-xs py-0.5">
            <CircleDashed size={12} className="animate-spin-slow mr-1.5" />
            <span>Agent synthesizing response...</span>
          </div>
        </div>
      );
    }

    return elements;
  };

  return (
    <>
      <div className="supervisor-stream-container flex-1" ref={scrollRef} onScroll={handleScroll}>
        <div className="supervisor-stream-content">
          {renderContent()}

          {(!activeThreadId || (threadMessages.length === 0 && threadIntercepts.length === 0 && !pendingText)) && (
            <div className="flex flex-col justify-center items-center py-20 text-slate-500 select-none">
              <CheckCircle2 size={32} className="mb-2 text-slate-600" />
              <div>Select or launch a thread to inspect logs.</div>
            </div>
          )}
        </div>
      </div>

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
                : "Enter directive or /mode, /model, /help..."
            }
            className="supervisor-input-field bg-[#0a0a0e] border border-white/10 rounded-xl py-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50 focus:shadow-[0_0_15px_rgba(6,182,212,0.15)] transition-all font-mono placeholder-slate-600"
            value={supervisorInput}
            onChange={(e) => setSupervisorInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && activeThreadId && onSendDirective(activeThreadId)}
            disabled={!activeThreadId || isPendingApproval}
          />
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
        <div className="flex items-center justify-center gap-3 mt-2 text-[10px] text-slate-600 font-mono">
          {activeThreadId
            ? (() => {
                const t = threads.find(t => t.id === activeThreadId);
                if (!t) return "No active thread";
                const statusMap: Record<string, string> = {
                  idle: "Idle",
                  running: "Running",
                  pending_approval: "Pending Approval",
                  spawning: "Spawning",
                  installing: "Installing",
                  error: "Error",
                };
                const label = statusMap[t.status] ?? t.status;
                return (
                  <>
                    <span>{label}</span>
                    <span className="text-slate-700">|</span>
                    <span className="text-slate-500">/mode /model /help</span>
                  </>
                );
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
          if (activeThreadId && rejectCardId) {
            onRejectAction(activeThreadId, rejectCardId, reason);
          }
          setRejectOpen(false);
          setRejectCardId(null);
        }}
        onCancel={() => {
          setRejectOpen(false);
          setRejectCardId(null);
        }}
      />
    </>
  );
};

export default SupervisorPlane;
