import React, { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { TerminalSquare } from "lucide-react";
import { useTerminalPanel } from "../context";

// Import XTerm styles
import "@xterm/xterm/css/xterm.css";

interface TerminalWindowProps {
  projectId: string;
  computeType: string;
  remoteHost: string | null;
  repoPath: string;
  /** Absolute path — forwarded to the panel as `workDir` so the shell
   *  opens inside the supplied directory (e.g. a feature worktree)
   *  instead of a basename-derived clone. */
  workDir?: string;
  /** Feature branch to `git checkout` after the PTY starts. Omit for
   *  `ProjectHome`-style terminals (no pipeline context). */
  workBranch?: string | null;
  /** Called once after the panel tab has been registered. The supplied
   *  id is the frontend-minted `tabId` (NOT the backend session id). */
  onSessionStarted?: (tabId: string) => void;
}

/**
 * Thin view that opens a panel tab on mount. The actual xterm canvas
 * and the backend session lifecycle now live in
 * `TerminalSurface` / `TerminalPanelProvider` (see
 * `src/context/TerminalPanelProvider.tsx`). The local xterm + FitAddon
 * instances below are kept solely to preserve the original component's
 * visible "terminal chrome" — the panel's surface is what renders the
 * real PTY output.
 */
export const TerminalWindow: React.FC<TerminalWindowProps> = ({
  projectId,
  computeType,
  remoteHost,
  repoPath,
  workDir: workDirProp,
  workBranch,
  onSessionStarted,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);

  const { open: openPanelTab } = useTerminalPanel();

  useEffect(() => {
    const machineId = computeType.toLowerCase() === "remote" ? remoteHost || "local" : "local";
    void openPanelTab({
      machineId,
      machineLabel: machineId,
      projectId,
      workDir: workDirProp,
      repoPath,
      workBranch: workBranch ?? null,
    }).then((tabId) => onSessionStarted?.(tabId));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: '"Fira Code", "JetBrains Mono", Menlo, Monaco, Consolas, monospace',
      theme: {
        background: "#08090c",
        foreground: "#cbd5e1",
        cursor: "#06b6d4",
        selectionBackground: "rgba(6, 182, 212, 0.3)",
        black: "#0f172a",
        red: "#ef4444",
        green: "#10b981",
        yellow: "#f59e0b",
        blue: "#3b82f6",
        magenta: "#8b5cf6",
        cyan: "#06b6d4",
        white: "#f8fafc",
      },
      allowProposedApi: true,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    term.open(containerRef.current);
    try {
      fitAddon.fit();
    } catch {
      // Initial fit on a 0-sized container can throw; the ResizeObserver
      // picks up the real geometry once flexbox lays the panel out.
    }

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    const observer = new ResizeObserver(() => {
      try {
        fitAddon.fit();
      } catch {
        // Container may briefly report 0×0 during layout transitions.
      }
    });
    observer.observe(containerRef.current);
    resizeObserverRef.current = observer;

    return () => {
      observer.disconnect();
      resizeObserverRef.current = null;
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, []);

  return (
    <div className="flex flex-col h-full w-full bg-[#050608] border border-white/5 rounded-xl overflow-hidden relative">
      <div className="px-4 py-2 bg-[#0c0d12] border-b border-white/5 flex items-center justify-between shrink-0 select-none">
        <div className="flex items-center gap-2">
          <TerminalSquare className="w-4 h-4 text-cyan-400" />
          <span className="text-xs font-mono text-slate-300">
            terminal://{computeType === "local" ? "local" : "remote"}/{repoPath}
          </span>
        </div>
        <span className="text-[10px] uppercase tracking-wider text-cyan-400 font-mono">
          See panel below
        </span>
      </div>
      <div className="flex-1 min-h-0 relative p-3 bg-[#08090c]">
        <div ref={containerRef} className="w-full h-full" />
      </div>
    </div>
  );
};
