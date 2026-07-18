import { useCallback, useEffect, useRef, useState } from 'react';
import { Channel } from '@tauri-apps/api/core';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { RotateCw, Wifi, WifiOff, AlertCircle } from 'lucide-react';

import '@xterm/xterm/css/xterm.css';

import {
  attachTerminalSession,
  detachTerminalSession,
  resizeTerminalSession,
  writeTerminalSession,
} from '../lib/terminal';
import { useTerminalPanel } from '../hooks/useTerminalPanel';

// xterm.js theme — matches the existing TerminalWindow palette so the
// panel surface and the legacy modal look identical (AGENTS.md §5).
const XTERM_THEME = {
  background: '#08090c',
  foreground: '#cbd5e1',
  cursor: '#06b6d4',
  selectionBackground: 'rgba(6, 182, 212, 0.3)',
  black: '#0f172a',
  red: '#ef4444',
  green: '#10b981',
  yellow: '#f59e0b',
  blue: '#3b82f6',
  magenta: '#8b5cf6',
  cyan: '#06b6d4',
  white: '#f8fafc',
} as const;

export interface TerminalSurfaceProps {
  /** Frontend-minted stable id (spec §7 Q1). Surfaced via
   *  `data-tab-id` so tests can target the surface without coupling to
   *  the backend session id. */
  tabId: string;
  /** Backend session id. The surface mounts only when this is non-null
   *  (the panel host gates the render); a remount on sessionId change
   *  rebinds to the new backend subscriber Vec. */
  sessionId: string;
  /** Lifecycle phase from the panel descriptor — used for the inline
   *  status overlay (no xterm canvas paints while we are connecting /
   *  closed / errored). */
  phase: 'connecting' | 'running' | 'closed' | 'error';
  /** User-facing title shown in the surface's local toolbar. */
  title: string;
  /** Host label (local / hostname) shown next to the title. */
  machineLabel: string;
}

/**
 * One xterm.js surface bound to one terminal session. Lifecycle:
 *
 *   1. Mount:    construct Terminal + FitAddon, open into the container,
 *                create a Tauri `Channel`, register `onmessage`, call
 *                `attach_terminal_session(sessionId, channel)`.
 *   2. Replay:   drain the panel's startup-output buffer and write the
 *                captured bytes to xterm so the shell prompt survives
 *                the gap between `start_terminal_session` resolving
 *                and the surface mounting (spec §1 AC #1).
 *   3. Resize:   `ResizeObserver` triggers `fitAddon.fit()` and
 *                `resize_terminal_session(sessionId, cols, rows)`.
 *   4. Unmount:  call `detach_terminal_session(sessionId, channelId)`
 *                and dispose the xterm instance. NEVER call
 *                `close_terminal_session` — that is reserved for
 *                explicit user action (close tab, kill all, tray
 *                `CloseAction::Cleanup`).
 *
 * The surface is purely a view: it does not own session teardown. It
 * is the visual half of the hook's lifecycle split.
 */
export function TerminalSurface({
  tabId,
  sessionId,
  phase,
  title,
  machineLabel,
}: TerminalSurfaceProps): React.ReactElement {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const sessionIdRef = useRef<string>(sessionId);
  const { consumeStartupReplay } = useTerminalPanel();

  // The first byte after a fresh attach can race the layout flush;
  // show a transient status badge so the user sees the panel has work
  // to do instead of staring at an empty canvas.
  const [bootstrapping, setBootstrapping] = useState(true);

  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  const handleResize = useCallback(() => {
    const term = terminalRef.current;
    const fit = fitAddonRef.current;
    if (!term || !fit) return;
    try {
      fit.fit();
      const cols = term.cols;
      const rows = term.rows;
      if (cols > 0 && rows > 0) {
        resizeTerminalSession(sessionIdRef.current, cols, rows).catch((err) => {
          console.warn('[TerminalSurface] resize_terminal_session failed:', err);
        });
      }
    } catch (err) {
      console.warn('[TerminalSurface] fit failed:', err);
    }
  }, []);

  useEffect(() => {
    if (!containerRef.current) return;
    const sid = sessionId;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: '"Fira Code", "JetBrains Mono", Menlo, Monaco, Consolas, monospace',
      theme: XTERM_THEME,
      allowProposedApi: true,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    term.open(containerRef.current);
    try {
      fitAddon.fit();
    } catch (err) {
      console.warn('[TerminalSurface] initial fit failed:', err);
    }

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;

    term.onData((data) => {
      void writeTerminalSession(sessionIdRef.current, data).catch((err) => {
        console.error('[TerminalSurface] write_terminal_session failed:', err);
      });
    });

    const observer = new ResizeObserver(() => handleResize());
    observer.observe(containerRef.current);
    resizeObserverRef.current = observer;

    const channel = new Channel<Uint8Array | number[]>();
    const channelId = channel.id;
    channel.onmessage = (chunk: Uint8Array | number[]) => {
      if (terminalRef.current) {
        const bytes = chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk);
        terminalRef.current.write(bytes);
      }
    };

    let cancelled = false;
    attachTerminalSession(sid, channel)
      .then(() => {
        if (cancelled) return;
        setBootstrapping(false);
        // Replay any startup bytes the panel's seed channel captured
        // between `start_terminal_session` resolving and this surface
        // mounting. Without this, the shell prompt and any
        // `git checkout` bootstrap output land in the seed buffer and
        // never reach xterm — the user sees an empty canvas.
        const replay = consumeStartupReplay(tabId);
        if (replay && replay.byteLength > 0 && terminalRef.current) {
          terminalRef.current.write(replay);
        }
        setTimeout(() => handleResize(), 50);
      })
      .catch((err) => {
        console.error('[TerminalSurface] attach_terminal_session failed:', err);
        if (!cancelled) {
          setBootstrapping(false);
        }
      });

    return () => {
      cancelled = true;
      observer.disconnect();
      resizeObserverRef.current = null;

      detachTerminalSession(sid, channelId).catch((err) => {
        console.warn('[TerminalSurface] detach_terminal_session failed:', err);
      });

      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, [sessionId, handleResize, tabId, consumeStartupReplay]);

  // Phase overlay badge. The xterm canvas keeps rendering underneath
  // — this is a small, non-blocking indicator for the four lifecycle
  // states (the running pulse lives in TerminalTab; the surface just
  // labels the current state).
  return (
    <div
      className="flex-1 min-h-0 flex flex-col bg-[#08090c]"
      data-testid="terminal-surface"
      data-tab-id={tabId}
      data-session-id={sessionId}
    >
      <div className="px-3 py-1.5 bg-[#0c0d12] border-b border-white/[0.05] flex items-center justify-between shrink-0">
        <div className="flex items-center gap-2 text-[11px] font-mono text-slate-400 truncate min-w-0">
          <span className="text-cyan-400 shrink-0">{machineLabel}</span>
          <span className="opacity-50 shrink-0">/</span>
          <span className="truncate">{title}</span>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          {phase === 'connecting' || bootstrapping ? (
            <span className="flex items-center gap-1.5 text-[10px] text-amber-400 font-mono">
              <RotateCw className="w-3 h-3 animate-spin" />
              <span>Connecting</span>
            </span>
          ) : phase === 'running' ? (
            <span className="flex items-center gap-1.5 text-[10px] text-emerald-400 font-mono">
              <Wifi className="w-3 h-3 animate-pulse" />
              <span>Connected</span>
            </span>
          ) : phase === 'closed' ? (
            <span className="flex items-center gap-1.5 text-[10px] text-slate-500 font-mono">
              <WifiOff className="w-3 h-3" />
              <span>Closed</span>
            </span>
          ) : (
            <span className="flex items-center gap-1.5 text-[10px] text-ruby-400 font-mono">
              <AlertCircle className="w-3 h-3" />
              <span>Error</span>
            </span>
          )}
        </div>
      </div>

      <div className="flex-1 min-h-0 relative p-3">
        <div ref={containerRef} className="w-full h-full" />
      </div>
    </div>
  );
}