import React, { useState, useEffect, useCallback } from 'react';
import { X, Terminal } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { writeTerminalSession } from '../lib/terminal';
import { useTerminalPanel } from '../context';

interface AgentEntry {
  kind: string;
  binary: string;
  label: string;
}

const AGENT_CLI: Record<string, { binary: string; label: string }> = {
  'claude-code': { binary: 'claude', label: 'Claude' },
  'opencode':    { binary: 'opencode', label: 'OpenCode' },
  'hermes':      { binary: 'hermes', label: 'Hermes' },
  'codex':       { binary: 'codex', label: 'Codex' },
};

export interface AgentTerminalDrawerProps {
  /**
   * Legacy `isOpen` prop kept for back-compat with existing call sites
   * that haven't migrated to the panel-driven launch flow. The drawer
   * no longer returns `null` based on this flag — it always renders as
   * a trigger surface (spec §3 (d)). The prop is ignored.
   * @deprecated route through `useTerminalPanel().open({...})` directly
   *   instead.
   */
  isOpen?: boolean;
  /** Legacy close callback, also ignored post-migration. */
  onClose?: () => void;
  /** machineId for agent config lookup and PTY routing */
  machineId: string;
  /** Pre-resolved absolute path (feature worktree). Skips resolveRepoDir. */
  absoluteWorkDir?: string;
  /** Project-relative repo path. Used when absoluteWorkDir is absent. */
  repoPath?: string;
  /** Project id — forwarded to the panel so the tab descriptor carries it. */
  projectId: string;
  /** Unused post-migration; the panel derives transport from `machineId`. */
  computeType?: string;
  /** Unused post-migration; kept for back-compat with existing mounts. */
  remoteHost?: string | null;
  /** Feature branch (`demeteo/features/<id>`) to auto-checkout after the
   *  PTY starts. Omit for terminal draws outside a feature context. */
  workBranch?: string | null;
  /** Sidebar width in px so the drawer doesn't overlap it. Default 240. */
  sidebarWidth?: number;
}

/**
 * Trigger surface for the per-machine agent launchers.
 *
 * Post-migration (spec §3 (d)), the drawer:
 *
 *   • never owns session teardown — the panel's tab-close button,
 *     kill-all affordance, or tray `CloseAction::Cleanup` are the
 *     only paths that tear down a session;
 *   • routes each Launch button through `useTerminalPanel().open(...)`
 *     so a panel tab appears the moment the user clicks Claude /
 *     OpenCode / Hermes / Codex. Once the tab is open the agent binary
 *     is forwarded into the new session via `write_terminal_session`.
 *
 * The `isOpen` → `return null` guard that previously killed the
 * underlying PTY the moment the drawer was closed has been retired.
 */
export const AgentTerminalDrawer: React.FC<AgentTerminalDrawerProps> = ({
  machineId,
  absoluteWorkDir,
  repoPath,
  projectId,
  workBranch,
}) => {
  const { open: openTerminalTab, getSessionId } = useTerminalPanel();
  const [cachedTabId, setCachedTabId] = useState<string | null>(null);
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [launching, setLaunching] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const configs = await invoke<Array<{ kind: string; enabled: boolean }>>('get_agent_configs', { machineId });
        const found = (configs || [])
          .filter(c => c.enabled && AGENT_CLI[c.kind])
          .map(c => ({ kind: c.kind, ...AGENT_CLI[c.kind] }));
        if (!cancelled) setAgents(found.length > 0 ? found : defaultAgents());
      } catch {
        if (!cancelled) setAgents(defaultAgents());
      }
    })();
    return () => { cancelled = true; };
  }, [machineId]);

  const handleLaunchAgent = useCallback(
    async (agent: AgentEntry) => {
      if (launching) return;
      setLaunching(agent.kind);
      try {
        // Open the panel tab on first launch. If a tab is already
        // alive for this machine + repo, the panel either re-focuses
        // the existing one or opens a fresh one — either way the user
        // sees the running PTY in the panel.
        const tabId = cachedTabId ?? await openTerminalTab({
          machineId,
          machineLabel: machineId,
          projectId,
          workDir: absoluteWorkDir,
          repoPath,
          workBranch,
        });
        setCachedTabId(tabId);
        // Resolve the backend-allocated `sess_*` id (NOT the tabId —
        // tabId is a frontend UUID the backend has never heard of).
        // open() awaits `start_terminal_session` internally, so by the
        // time it returned the binding is already populated.
        const sessionId = getSessionId(tabId);
        if (!sessionId) {
          console.warn(
            '[AgentTerminalDrawer] no sessionId resolved for tabId',
            tabId,
            '— the panel session may have failed to start',
          );
          return;
        }
        // Forward the agent binary into the now-running PTY. The
        // panel owns the channel; we just feed keystrokes.
        await writeTerminalSession(sessionId, agent.binary + '\r');
      } catch (err) {
        console.warn('AgentTerminalDrawer: failed to launch agent', err);
      } finally {
        setLaunching(null);
      }
    },
    [
      cachedTabId,
      launching,
      openTerminalTab,
      getSessionId,
      machineId,
      projectId,
      absoluteWorkDir,
      repoPath,
      workBranch,
    ],
  );

  const pathLabel = absoluteWorkDir
    ? absoluteWorkDir.split('/').slice(-2).join('/')
    : repoPath || projectId;

  // The drawer no longer returns null — it always renders as a
  // trigger surface. The legacy `isOpen` prop is ignored.
  return (
    <div
      data-testid="agent-terminal-drawer"
      data-legacy-trigger="true"
      className="flex items-center gap-3 px-3 py-2 rounded-lg border border-white/5 bg-[#0c0d12]/80 backdrop-blur-md text-[11px] font-mono text-slate-400"
    >
      <Terminal className="w-3.5 h-3.5 text-cyan-400 shrink-0" />
      <span className="truncate max-w-[240px]" title={pathLabel}>{pathLabel}</span>

      <div className="h-3.5 w-px bg-white/10 mx-1 shrink-0" />

      {/* Agent launch buttons */}
      <div className="flex items-center gap-2 flex-1 min-w-0">
        <span className="text-[10px] text-slate-500 uppercase font-bold tracking-wider shrink-0">
          Launch
        </span>
        {agents.map(agent => (
          <button
            key={agent.kind}
            onClick={() => { void handleLaunchAgent(agent); }}
            disabled={launching !== null}
            title={`Run ${agent.binary} in a new terminal tab — opens in the global panel`}
            className={`px-3 py-1 rounded text-[10px] font-bold uppercase tracking-wider border transition-all shrink-0
              disabled:opacity-30 disabled:cursor-not-allowed
              ${launching === agent.kind
                ? 'bg-cyan-500/20 border-cyan-400/60 text-cyan-300 animate-pulse'
                : 'bg-white/5 border-white/10 text-slate-300 hover:bg-cyan-500/15 hover:border-cyan-500/40 hover:text-cyan-300'
              }`}
          >
            {agent.label}
          </button>
        ))}
      </div>

      <button
        onClick={() => { /* legacy prop — drawer no longer owns the close lifecycle */ }}
        className="ml-auto p-1.5 rounded-lg text-slate-500 hover:text-white hover:bg-white/5 transition shrink-0"
        title="(legacy) Close is owned by the panel — close the active tab instead"
        aria-label="Close terminal drawer (no-op)"
      >
        <X className="w-4 h-4" />
      </button>
    </div>
  );
};

function defaultAgents(): AgentEntry[] {
  return [
    { kind: 'claude-code', ...AGENT_CLI['claude-code'] },
    { kind: 'opencode', ...AGENT_CLI['opencode'] },
  ];
}