import React, { useCallback, useEffect, useRef, useState } from 'react';
import { TerminalSquare, ChevronDown, Sparkles } from 'lucide-react';

import { useTerminalPanel } from '../hooks/useTerminalPanel';
import { useNavigation } from '../context';
import { AGENTS, type AgentMeta } from '../lib/agents';
import { recordRecent } from '../lib/terminalRecents';
import { formatError } from '../lib/errors';

export interface StartSessionButtonProps {
  projectId: string;
  repoPath: string;
  machineId: string;
  machineLabel: string;
  className?: string;
}

/**
 * Primary "start a session" split button for Project Home.
 *
 * The primary button opens a plain shell scoped to `repoPath` on
 * `machineId`; the caret lists the coding agents from `../lib/agents` so
 * one can be launched straight into a fresh session instead. Mirrors the
 * split-button pattern in `NewTerminalMenu`, but the caller (not this
 * component) resolves which machine/repo to target — this is just the
 * launcher for an already-known target.
 *
 * Every open uses `forceNew: true`: this is an explicit, repeatable user
 * action, so repeated clicks stack new sessions rather than refocusing an
 * existing tab (unlike the passive `TerminalTabOpener` route effect in
 * `ProjectHome.tsx`).
 */
export function StartSessionButton({
  projectId,
  repoPath,
  machineId,
  machineLabel,
  className = '',
}: StartSessionButtonProps): React.ReactElement {
  const { open } = useTerminalPanel();
  const { navigate } = useNavigation();

  const [menuOpen, setMenuOpen] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const containerRef = useRef<HTMLDivElement | null>(null);
  // Latch for the in-flight guard. `launching` drives the disabled styling,
  // but reading it in `launch()` only reflects the last committed render, so
  // two clicks dispatched before React commits would both get through and —
  // with `forceNew: true` bypassing the panel's pending-open coalescing —
  // spawn two PTYs for one perceived click. The ref is set synchronously.
  const launchingRef = useRef(false);

  // Close the agent dropdown on an outside click or Escape, mirroring
  // NewTerminalMenu's popover behavior.
  useEffect(() => {
    if (!menuOpen) return;
    const onDocClick = (e: globalThis.MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    };
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') setMenuOpen(false);
    };
    document.addEventListener('mousedown', onDocClick);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onDocClick);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [menuOpen]);

  const launch = useCallback(
    async (agent?: AgentMeta) => {
      // Single in-flight guard so a double-click (or a click while an
      // earlier launch is still resolving) can't stack duplicate sessions
      // from what the user experiences as one click.
      if (launchingRef.current) return;
      // No repo path resolved yet (workspace still loading, or the project has
      // no repositories) — same guard `TerminalTabOpener` uses, so a click in
      // that window can't open a session at the process's default directory
      // instead of the repo.
      if (!repoPath) return;
      launchingRef.current = true;
      setLaunching(true);
      setMenuOpen(false);
      setError(null);
      try {
        await open({
          machineId,
          machineLabel,
          projectId,
          repoPath,
          forceNew: true,
          agentKind: agent?.kind ?? null,
          launchCommand: agent?.binary,
        });
        recordRecent({ machineId, machineLabel, agentKind: agent?.kind ?? null });
        navigate({ kind: 'terminals' });
      } catch (err) {
        setError(formatError(err));
      } finally {
        launchingRef.current = false;
        setLaunching(false);
      }
    },
    [open, machineId, machineLabel, projectId, repoPath, navigate],
  );

  // A stale failure must not follow the user to another project or repo: the
  // message names a target that is no longer the one this button would open.
  useEffect(() => {
    setError(null);
  }, [projectId, repoPath, machineId]);

  // Disabled while a launch is in flight, and until the workspace has
  // resolved a repo path to scope the session to.
  const disabled = launching || !repoPath;

  return (
    <div ref={containerRef} className={`relative ${className}`} data-testid="start-session-button">
      <div className="inline-flex shrink-0 rounded-md shadow-sm">
        <button
          type="button"
          onClick={() => void launch()}
          disabled={disabled}
          className="flex items-center gap-1.5 pl-3 pr-3 py-1.5 rounded-l-md text-xs font-mono whitespace-nowrap border border-white/10 border-r-0 bg-violet-600 hover:bg-violet-500 text-white shadow-[0_0_15px_rgba(139,92,246,0.4)] transition-all disabled:opacity-40"
          title={repoPath ? 'Start a shell session in this repo' : 'No repository resolved for this workspace yet'}
          data-testid="start-session-primary"
        >
          <TerminalSquare className="w-3.5 h-3.5 shrink-0" />
          <span>Start session</span>
        </button>
        <button
          type="button"
          onClick={() => setMenuOpen((v) => !v)}
          disabled={disabled}
          className="flex items-center px-1.5 py-1.5 rounded-r-md text-xs font-mono border border-white/10 bg-violet-600 hover:bg-violet-500 text-white transition-all disabled:opacity-40"
          title="Choose an agent to launch"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          data-testid="start-session-caret"
        >
          <ChevronDown className="w-3 h-3 opacity-80" />
        </button>
      </div>

      {menuOpen && (
        <div
          role="menu"
          className="absolute left-0 mt-1 z-30 w-52 rounded-lg border border-white/10 bg-[#0c0d12] shadow-2xl overflow-hidden p-1"
          data-testid="start-session-dropdown"
        >
          {Object.values(AGENTS).map((agent) => (
            <button
              key={agent.kind}
              type="button"
              role="menuitem"
              onClick={() => void launch(agent)}
              disabled={disabled}
              className="w-full flex items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-[12px] font-mono text-slate-300 hover:bg-violet-500/15 hover:text-violet-200 transition disabled:opacity-40"
              title={`Run ${agent.binary} in a new session`}
              data-testid={`start-session-agent-${agent.kind}`}
            >
              <Sparkles className="w-3.5 h-3.5 text-violet-400 shrink-0" />
              <span>{agent.label}</span>
            </button>
          ))}
        </div>
      )}

      {error && (
        <div
          className="absolute left-0 top-full mt-1.5 z-20 max-w-xs rounded-md border border-ruby-500/30 bg-ruby-500/10 px-2 py-1 text-[11px] font-mono text-ruby-300"
          data-testid="start-session-error"
        >
          {error}
        </div>
      )}
    </div>
  );
}
