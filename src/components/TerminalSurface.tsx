import { useCallback, useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { RotateCw, Wifi, WifiOff, AlertCircle } from 'lucide-react';

import '@xterm/xterm/css/xterm.css';

import {
  attachTerminalSession,
  createTerminalChannel,
  detachTerminalSession,
  resizeTerminalSession,
  writeTerminalSession,
} from '../lib/terminal';
import {
  getLastTerminalSize,
  hasLayoutBox,
  isPlausibleTerminalSize,
  setLastTerminalSize,
} from '../lib/terminalViewport';
import { MachineDot } from './ui/MachineDot';
import { AgentBadge } from './ui/AgentBadge';
import { ActivityIndicator } from './ui/ActivityIndicator';
import type { TerminalActivity } from '../types';

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
   *  disconnected / closed / errored). */
  phase: 'connecting' | 'running' | 'disconnected' | 'closed' | 'error';
  /** User-facing title shown in the surface's local toolbar. */
  title: string;
  /** Host label (local / hostname) shown next to the title. */
  machineLabel: string;
  /** Machine id — drives the local/remote dot colour in the header. */
  machineId: string;
  /** Coding-agent kind running in the session, or null for a plain shell. */
  agentKind?: string | null;
  /** Live activity of the agent in the focused session, or null when there
   *  is no signal. Renders the same mark shown on the session-list row. */
  activity?: TerminalActivity;
  /** False while the surface sits in a `display:none` subtree — i.e. the
   *  Terminals route is not the active view and the surface stays mounted
   *  behind it. Defaults to true so every existing caller keeps its current
   *  behaviour. */
  visible?: boolean;
}

/**
 * One xterm.js surface bound to one terminal session. Lifecycle:
 *
 *   1. Mount:    construct Terminal + FitAddon, open into the container,
 *                create an output channel, register `onmessage`, call
 *                `attach_terminal_session(sessionId, channel)`.
 *   2. Replay:   the backend replays the session's scrollback ring to
 *                the freshly-attached channel, so the shell prompt and
 *                any pre-attach output survive the gap between
 *                `start_terminal_session` resolving and the surface
 *                mounting (TERMINALS_VIEW_SPEC §3). No frontend buffer.
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
  machineId,
  agentKind,
  activity,
  visible = true,
}: TerminalSurfaceProps): React.ReactElement {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const resizeObserverRef = useRef<ResizeObserver | null>(null);
  const sessionIdRef = useRef<string>(sessionId);
  // The last (cols, rows) actually pushed to the backend PTY. Seeded from the
  // shared size cache — i.e. the size the session was *spawned* at — so the
  // first post-attach fit that lands on the same size skips the resize
  // entirely. Sending a redundant `SIGWINCH` during P10k's instant/transient
  // prompt startup is what duplicated the command line (see terminalViewport).
  const lastSentSizeRef = useRef<{ cols: number; rows: number } | null>(null);
  const wasVisibleRef = useRef<boolean>(visible);

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
    // A fit taken with no layout box measures the *computed* value of the
    // `w-full`/`h-full` container — the literal string "100%", which FitAddon's
    // `proposeDimensions()` parseInts into 100 pixels. At `fontSize: 13` that is
    // a plausible-looking 11 × 5 which would then be cached and pushed to the
    // PTY. Silent, not warned: the observer keeps ticking the whole time the
    // Terminals route is hidden.
    if (!hasLayoutBox(containerRef.current)) return;
    try {
      fit.fit();
      const cols = term.cols;
      const rows = term.rows;
      if (!isPlausibleTerminalSize(cols, rows)) {
        console.warn('[TerminalSurface] implausible fit, ignoring:', cols, rows);
        return;
      }
      // Cache the fitted size so the next session `open()` can spawn its PTY
      // at the real width, drawing its first prompt at the correct size.
      setLastTerminalSize(cols, rows);
      // Skip the PTY resize when nothing changed. This suppresses the
      // redundant startup `SIGWINCH` that corrupts P10k's instant/transient
      // prompt redraw (duplicated command line): a session spawned at the
      // cached width fits to that same width and never resizes at all.
      const last = lastSentSizeRef.current;
      if (last && last.cols === cols && last.rows === rows) return;
      lastSentSizeRef.current = { cols, rows };
      resizeTerminalSession(sessionIdRef.current, cols, rows).catch((err) => {
        console.warn('[TerminalSurface] resize_terminal_session failed:', err);
      });
    } catch (err) {
      console.warn('[TerminalSurface] fit failed:', err);
    }
  }, []);

  useEffect(() => {
    if (!containerRef.current) return;
    const sid = sessionId;
    // Seed the "last sent" size with the size the session was spawned at (the
    // shared cache value `open()` passed to `start_terminal_session`). When
    // this surface fits to that same size, `handleResize` recognises the match
    // and sends no PTY resize — keeping P10k's startup SIGWINCH-free.
    lastSentSizeRef.current = getLastTerminalSize();

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      // Per-glyph browser fallback walks this list left→right: text glyphs
      // resolve from the first Nerd/text font present, and prompt icon /
      // powerline glyphs (P10k separators, timestamp frame) fall through to
      // the bundled "Symbols Nerd Font Mono" face (App.css @font-face) when no
      // system Nerd Font is installed — otherwise they render as `?`-tofu.
      fontFamily:
        '"MesloLGS NF", "FiraCode Nerd Font", "JetBrainsMono Nerd Font", "Hack Nerd Font", ' +
        '"Fira Code", "Symbols Nerd Font Mono", Menlo, Monaco, Consolas, monospace',
      theme: XTERM_THEME,
      allowProposedApi: true,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    term.open(containerRef.current);

    // GPU-accelerated renderer. xterm's default DOM renderer redraws glyphs via
    // CPU DOM manipulation, which shows up as sustained WebContent/GPU CPU under
    // heavy streaming output; the WebGL renderer uploads the cell grid to the
    // GPU as a single shader program instead. Must load AFTER term.open(). If
    // WebGL is unavailable (or the context is later lost — e.g. GPU reset, tab
    // backgrounded too long), dispose the addon and let xterm fall back to the
    // DOM renderer automatically. Never let a renderer failure kill the surface.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        console.warn('[TerminalSurface] WebGL context lost, falling back to DOM renderer');
        webgl.dispose();
        // Context loss wipes the GPU canvas's pixels; disposing the addon
        // hands rendering back to xterm's DOM renderer, but nothing dirties
        // the existing rows, so the last-painted screen would otherwise
        // stay blank until unrelated output happened to touch each row. Force
        // a full repaint of the current viewport now so the DOM renderer
        // redraws what's already in the buffer immediately.
        term.refresh(0, term.rows - 1);
      });
      term.loadAddon(webgl);
    } catch (err) {
      console.warn('[TerminalSurface] WebGL renderer unavailable, using DOM renderer:', err);
    }

    try {
      if (hasLayoutBox(containerRef.current)) fitAddon.fit();
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

    const channel = createTerminalChannel();
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
        // The backend replays the session's scrollback ring to this
        // freshly-attached channel (TERMINALS_VIEW_SPEC §3), so any
        // pre-attach output — shell prompt, `git checkout` bootstrap —
        // arrives through the normal `onmessage` path above. No frontend
        // replay buffer to drain here.
        //
        // Wait for web fonts before the reconciling fit so xterm measures the
        // cell from the final (Nerd) font metrics — an early fit on fallback
        // metrics could compute a different column count than the spawn size
        // and trigger an otherwise-avoidable resize. `document.fonts.ready`
        // resolves immediately once fonts are loaded.
        void document.fonts.ready.then(() => {
          if (!cancelled) handleResize();
        });
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
  }, [sessionId, handleResize]);

  // Returning from a `display:none` subtree is the same class of renderer
  // discontinuity as the lost WebGL context above: nothing dirties the rows, so
  // the stale frame stays until unrelated output happens to touch each one.
  // The repaint belongs here and not in `handleResize`, which returns at the
  // `lastSentSizeRef` equality check precisely when the size did *not* change —
  // the common case on the way back. So: always re-fit and repaint locally on
  // the edge, still resize the PTY only on a genuine change. Resizing
  // unconditionally on show puts a `SIGWINCH` back into P10k's startup and
  // duplicates the command line again (see terminalViewport).
  useEffect(() => {
    const wasVisible = wasVisibleRef.current;
    wasVisibleRef.current = visible;
    if (!visible || wasVisible) return;
    const term = terminalRef.current;
    if (!term) return;
    handleResize();
    term.refresh(0, term.rows - 1);
  }, [visible, handleResize]);

  // On-screen "needs a decision" recognition (Phase 3) lives in the
  // always-mounted `TerminalApprovalRecognizer`, not here: only the focused tab
  // mounts a surface, so recognizing off this buffer alone could never see a
  // backgrounded agent's approval prompt — which is the whole point. The
  // recognizer keeps a headless buffer per agent session instead, so this
  // surface stays purely presentational.

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
          <MachineDot machineId={machineId} machineLabel={machineLabel} pulse={phase === 'running'} />
          <span className="text-cyan-400 shrink-0">{machineLabel}</span>
          <span className="opacity-50 shrink-0">/</span>
          <span className="truncate">{title}</span>
          <AgentBadge agentKind={agentKind} className="ml-1 shrink-0" />
          <ActivityIndicator activity={activity ?? null} className="ml-1 shrink-0" />
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
          ) : phase === 'disconnected' ? (
            <span className="flex items-center gap-1.5 text-[10px] text-amber-400 font-mono">
              <WifiOff className="w-3 h-3" />
              <span>Disconnected</span>
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