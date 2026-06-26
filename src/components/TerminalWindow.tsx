import React, { useEffect, useRef, useState } from "react";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { formatError } from "../lib/errors";
import { Channel } from "@tauri-apps/api/core";
import { TerminalSquare, RotateCw, AlertCircle, Wifi, WifiOff } from "lucide-react";
import {
  startTerminalSession,
  writeTerminalSession,
  resizeTerminalSession,
  closeTerminalSession,
  resolveRepoDir,
} from "../lib/terminal";

// Import XTerm styles
import "@xterm/xterm/css/xterm.css";

interface TerminalWindowProps {
  projectId: string;
  computeType: string;
  remoteHost: string | null;
  repoPath: string;
  /** Absolute path — skips resolveRepoDir when provided (e.g. feature worktrees) */
  workDir?: string;
  /** Called once after the PTY session connects and is ready for input */
  onSessionStarted?: (sessionId: string) => void;
}

export const TerminalWindow: React.FC<TerminalWindowProps> = ({
  projectId,
  computeType,
  remoteHost,
  repoPath,
  workDir: workDirProp,
  onSessionStarted,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);

  const [status, setStatus] = useState<"connecting" | "connected" | "disconnected" | "error">(
    "connecting"
  );
  const [errorDetail, setErrorDetail] = useState("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const activeSessionIdRef = useRef<string | null>(null);

  // Sync ref with state so async event listeners can read it
  useEffect(() => {
    activeSessionIdRef.current = sessionId;
  }, [sessionId]);

  useTauriEvent<{ session_id: string }>("terminal-session-ended", ({ session_id }) => {
    if (session_id === activeSessionIdRef.current) {
      setStatus("disconnected");
      terminalRef.current?.writeln("\r\n\x1b[1;33mTerminal session closed.\x1b[0m\r\n");
    }
  });

  const initTerminalSession = async () => {
    setStatus("connecting");
    setErrorDetail("");
    
    // Clean up any existing session first
    if (activeSessionIdRef.current) {
      try {
        await closeTerminalSession(activeSessionIdRef.current);
      } catch (e) {
        console.warn("Failed to clean up old session:", e);
      }
      setSessionId(null);
    }

    // Clear terminal screen if instantiated
    if (terminalRef.current) {
      terminalRef.current.clear();
      terminalRef.current.writeln("\x1b[1;36m>>> Connecting to terminal session...\x1b[0m");
    }

    try {
      // 1. Resolve work directory path
      const workDir = workDirProp ?? await resolveRepoDir(projectId, repoPath);

      // 2. Instantiate Tauri Channel for streaming stdout
      const channel = new Channel<Uint8Array | number[]>();
      channel.onmessage = (chunk: Uint8Array | number[]) => {
        if (terminalRef.current) {
          const bytes = chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk);
          terminalRef.current.write(bytes);
        }
      };

      // 3. Resolve the target machine string
      const machineId = computeType.toLowerCase() === "remote" ? remoteHost || "local" : "local";

      // 4. Start terminal session on the backend
      const sessId = await startTerminalSession(machineId, channel, workDir);
      setSessionId(sessId);
      setStatus("connected");
      onSessionStarted?.(sessId);

      if (terminalRef.current) {
        terminalRef.current.clear();
      }

      // 5. Initial resize synchronization
      setTimeout(() => {
        handleResize(sessId);
      }, 100);

    } catch (err: any) {
      console.error("Failed to start terminal session:", err);
      setStatus("error");
      setErrorDetail(formatError(err));
      if (terminalRef.current) {
        terminalRef.current.writeln(`\r\n\x1b[1;31mConnection error: ${err}\x1b[0m\r\n`);
      }
    }
  };

  const handleResize = (currentSessId: string | null) => {
    if (!currentSessId || !terminalRef.current || !fitAddonRef.current) return;
    try {
      fitAddonRef.current.fit();
      const cols = terminalRef.current.cols;
      const rows = terminalRef.current.rows;
      if (cols > 0 && rows > 0) {
        resizeTerminalSession(currentSessId, cols, rows).catch((e) => {
          console.warn("Failed to sync terminal resize:", e);
        });
      }
    } catch (e) {
      console.warn("Error during fit:", e);
    }
  };

  // Initialize terminal component instance
  useEffect(() => {
    if (!containerRef.current) return;

    // Create the XTerm Terminal instance
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

    // Mount terminal into div
    term.open(containerRef.current);
    fitAddon.fit();

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    // Bind terminal user input to backend
    term.onData((data) => {
      if (activeSessionIdRef.current) {
        writeTerminalSession(activeSessionIdRef.current, data).catch((e) => {
          console.error("Failed to write input:", e);
        });
      }
    });

    // Set up ResizeObserver to scale terminal with the container
    const observer = new ResizeObserver(() => {
      if (activeSessionIdRef.current) {
        handleResize(activeSessionIdRef.current);
      }
    });
    observer.observe(containerRef.current);
    resizeObserverRef.current = observer;

    // Initialize the backend session
    initTerminalSession();

    // Clean up terminal on unmount
    return () => {
      observer.disconnect();
      
      const sessToClose = activeSessionIdRef.current;
      if (sessToClose) {
        closeTerminalSession(sessToClose).catch((e) => {
          console.warn("Failed to close session on unmount:", e);
        });
      }

      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, [projectId, repoPath]);

  return (
    <div className="flex flex-col h-full w-full bg-[#050608] border border-white/5 rounded-xl overflow-hidden relative">
      {/* Terminal Viewport Toolbar */}
      <div className="px-4 py-2 bg-[#0c0d12] border-b border-white/5 flex items-center justify-between shrink-0 select-none">
        <div className="flex items-center gap-2">
          <TerminalSquare className="w-4 h-4 text-cyan-400" />
          <span className="text-xs font-mono text-slate-300">
            terminal://{computeType === "local" ? "local" : "remote"}/{repoPath}
          </span>
        </div>

        <div className="flex items-center gap-3">
          {/* Status Indicator */}
          {status === "connecting" && (
            <div className="flex items-center gap-1.5 text-xs text-amber-400 font-mono">
              <RotateCw className="w-3 h-3 animate-spin" />
              <span>Connecting</span>
            </div>
          )}
          {status === "connected" && (
            <div className="flex items-center gap-1.5 text-xs text-emerald-400 font-mono">
              <Wifi className="w-3.5 h-3.5 animate-pulse" />
              <span>Connected</span>
            </div>
          )}
          {status === "disconnected" && (
            <div className="flex items-center gap-1.5 text-xs text-slate-500 font-mono">
              <WifiOff className="w-3.5 h-3.5" />
              <span>Closed</span>
            </div>
          )}
          {status === "error" && (
            <div className="flex items-center gap-1.5 text-xs text-ruby-400 font-mono" title={errorDetail}>
              <AlertCircle className="w-3.5 h-3.5" />
              <span>Connection Failed</span>
            </div>
          )}

          {/* Reconnect Button */}
          {status !== "connecting" && (
            <button
              onClick={initTerminalSession}
              className="px-2 py-1 bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded text-[10px] font-mono transition-all flex items-center gap-1"
            >
              <RotateCw className="w-2.5 h-2.5" /> Reconnect
            </button>
          )}
        </div>
      </div>

      {/* Terminal Container */}
      <div className="flex-1 min-h-0 relative p-3 bg-[#08090c]">
        <div ref={containerRef} className="w-full h-full" />
      </div>
    </div>
  );
};
