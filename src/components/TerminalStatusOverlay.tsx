import React from "react";
import { Shield, KeyRound, Radio, TerminalSquare, AlertTriangle, CheckCircle2 } from "lucide-react";

type Phase = "resolving" | "connecting" | "authenticating" | "ready" | "error";

interface TerminalStatusOverlayProps {
  phase: Phase;
  host?: string;
  detail?: string;
}

const PHASES: { id: Phase; label: string; icon: React.ReactNode }[] = [
  { id: "resolving", label: "Resolving target", icon: <Radio size={12} /> },
  { id: "connecting", label: "Opening TCP tunnel", icon: <Shield size={12} /> },
  { id: "authenticating", label: "Authenticating", icon: <KeyRound size={12} /> },
  { id: "ready", label: "Session ready", icon: <TerminalSquare size={12} /> },
];

const TerminalStatusOverlay: React.FC<TerminalStatusOverlayProps> = ({ phase, host, detail }) => {
  if (phase === "ready") return null;

  const isError = phase === "error";
  const currentIndex = PHASES.findIndex((p) => p.id === phase);

  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-[#050508]/70 backdrop-blur-[2px] animate-fade-in pointer-events-none">
      <div className="pointer-events-auto min-w-[320px] max-w-[480px] rounded-xl border border-white/10 bg-[#0a0a0e]/95 shadow-2xl shadow-cyan-500/5 overflow-hidden">
        <div className="px-4 py-3 border-b border-white/5 flex items-center gap-2.5">
          {isError ? (
            <div className="w-2 h-2 rounded-full bg-red-500 shadow-[0_0_8px_2px_rgba(239,68,68,0.6)]" />
          ) : (
            <div className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse-glow" />
          )}
          <span className="text-[11px] font-mono uppercase tracking-[0.15em] text-slate-300">
            {isError ? "Connection failed" : "Establishing session"}
          </span>
          {host && !isError && (
            <span className="ml-auto text-[10px] font-mono text-slate-500 truncate max-w-[180px]">
              {host}
            </span>
          )}
        </div>

        <div className="p-4 space-y-3">
          {isError ? (
            <div className="flex gap-2.5">
              <AlertTriangle size={14} className="text-red-400 flex-shrink-0 mt-0.5" />
              <div className="text-xs text-slate-300 leading-relaxed">
                {detail || "Unable to establish SSH session."}
              </div>
            </div>
          ) : (
            <>
              <div className="flex items-center justify-center gap-2 py-2">
                {[0, 1, 2].map((i) => (
                  <span
                    key={i}
                    className={`w-2 h-2 rounded-full bg-cyan-400 animate-pulse-glow ${
                      i === 1 ? "animate-pulse-glow-delay-1" : i === 2 ? "animate-pulse-glow-delay-2" : ""
                    }`}
                  />
                ))}
              </div>

              <div className="space-y-1.5">
                {PHASES.map((p, idx) => {
                  const done = idx < currentIndex;
                  const active = idx === currentIndex;
                  return (
                    <div
                      key={p.id}
                      className={`flex items-center gap-2 text-[11px] font-mono transition-colors duration-300 ${
                        done
                          ? "text-emerald-400"
                          : active
                          ? "text-cyan-300"
                          : "text-slate-600"
                      }`}
                    >
                      <span className="w-4 flex justify-center">
                        {done ? (
                          <CheckCircle2 size={12} />
                        ) : active ? (
                          <span className="block w-1.5 h-1.5 rounded-full bg-cyan-400 shadow-[0_0_6px_1px_rgba(6,182,212,0.7)]" />
                        ) : (
                          <span className="block w-1.5 h-1.5 rounded-full bg-slate-700" />
                        )}
                      </span>
                      <span className="flex items-center gap-1.5">
                        {p.icon}
                        {p.label}
                      </span>
                    </div>
                  );
                })}
              </div>
            </>
          )}
        </div>

        {!isError && (
          <div className="h-0.5 animate-progress-shimmer" />
        )}
      </div>
    </div>
  );
};

export default TerminalStatusOverlay;
