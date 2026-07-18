import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Plus, TerminalSquare, ChevronDown } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';

import { useTerminalPanel } from '../hooks/useTerminalPanel';

interface AgentEntry {
  kind: string;
  binary: string;
  label: string;
}

/** Coding-agent CLIs the menu can launch into a fresh tab (spec §7).
 *  Mirrors the retiring `AgentTerminalDrawer`'s `AGENT_CLI`. */
const AGENT_CLI: Record<string, { binary: string; label: string }> = {
  'claude-code': { binary: 'claude', label: 'Claude' },
  opencode: { binary: 'opencode', label: 'OpenCode' },
  hermes: { binary: 'hermes', label: 'Hermes' },
  codex: { binary: 'codex', label: 'Codex' },
};

function defaultAgents(): AgentEntry[] {
  return [
    { kind: 'claude-code', ...AGENT_CLI['claude-code'] },
    { kind: 'opencode', ...AGENT_CLI['opencode'] },
  ];
}

export interface NewTerminalMenuProps {
  /** Machine to open sessions on. Defaults to the local host. */
  machineId?: string;
  machineLabel?: string;
  className?: string;
}

/**
 * Dropdown that opens a new terminal tab on a machine — either a bare
 * shell or a coding agent launched straight into the fresh session
 * (spec §5, §7). Every open uses `forceNew` so the menu can stack
 * multiple sessions on the same machine (auto-openers keep deduping).
 * Absorbs the agent-config loading the retired `AgentTerminalDrawer`
 * owned (finding F4).
 */
export function NewTerminalMenu({
  machineId = 'local',
  machineLabel = 'local',
  className = '',
}: NewTerminalMenuProps): React.ReactElement {
  const { open } = useTerminalPanel();
  const [openMenu, setOpenMenu] = useState(false);
  const [agents, setAgents] = useState<AgentEntry[]>(defaultAgents());
  const [launching, setLaunching] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const configs = await invoke<Array<{ kind: string; enabled: boolean }>>(
          'get_agent_configs',
          { machineId },
        );
        const found = (configs || [])
          .filter((c) => c.enabled && AGENT_CLI[c.kind])
          .map((c) => ({ kind: c.kind, ...AGENT_CLI[c.kind] }));
        if (!cancelled) setAgents(found.length > 0 ? found : defaultAgents());
      } catch {
        if (!cancelled) setAgents(defaultAgents());
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [machineId]);

  // While open: close on an outside click or Escape, and move focus onto
  // the first menu item so keyboard users can operate the menu (it was
  // previously mouse-only despite advertising `aria-haspopup="menu"`).
  useEffect(() => {
    if (!openMenu) return;
    const onDocClick = (e: globalThis.MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpenMenu(false);
      }
    };
    const onKeyDown = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        setOpenMenu(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener('mousedown', onDocClick);
    document.addEventListener('keydown', onKeyDown);
    menuRef.current
      ?.querySelector<HTMLElement>('[role="menuitem"]')
      ?.focus();
    return () => {
      document.removeEventListener('mousedown', onDocClick);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [openMenu]);

  const launch = useCallback(
    async (launchCommand?: string) => {
      if (launching) return;
      setLaunching(true);
      setOpenMenu(false);
      try {
        await open({ machineId, machineLabel, forceNew: true, launchCommand });
      } catch (err) {
        console.warn('[NewTerminalMenu] open failed:', err);
      } finally {
        setLaunching(false);
      }
    },
    [launching, open, machineId, machineLabel],
  );

  return (
    <div ref={containerRef} className={`relative ${className}`} data-testid="new-terminal-menu">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpenMenu((v) => !v)}
        disabled={launching}
        className="flex items-center gap-1.5 px-2.5 py-1 rounded-md text-[11px] font-mono border border-white/10 bg-white/5 text-slate-300 hover:bg-cyan-500/15 hover:border-cyan-500/40 hover:text-cyan-300 transition disabled:opacity-40"
        title="Open a new terminal"
        aria-haspopup="menu"
        aria-expanded={openMenu}
        data-testid="new-terminal-trigger"
      >
        <Plus className="w-3.5 h-3.5" />
        <span>New</span>
        <ChevronDown className="w-3 h-3 opacity-70" />
      </button>

      {openMenu && (
        <div
          ref={menuRef}
          role="menu"
          className="absolute right-0 mt-1 z-30 min-w-[180px] rounded-lg border border-white/10 bg-[#0c0d12] shadow-xl py-1"
          data-testid="new-terminal-dropdown"
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => void launch()}
            className="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] font-mono text-slate-300 hover:bg-white/5 hover:text-white transition text-left"
          >
            <TerminalSquare className="w-3.5 h-3.5 text-cyan-400 shrink-0" />
            <span>New shell</span>
          </button>

          {agents.length > 0 && (
            <>
              <div className="my-1 border-t border-white/5" />
              <div className="px-3 py-1 text-[9px] uppercase tracking-wider text-slate-600 font-bold">
                Agents
              </div>
              {agents.map((agent) => (
                <button
                  key={agent.kind}
                  type="button"
                  role="menuitem"
                  onClick={() => void launch(agent.binary)}
                  className="w-full flex items-center gap-2 px-3 py-1.5 text-[11px] font-mono text-slate-300 hover:bg-cyan-500/15 hover:text-cyan-300 transition text-left"
                  title={`Run ${agent.binary} in a new terminal on ${machineLabel}`}
                >
                  <span className="w-3.5 h-3.5 shrink-0" />
                  <span>{agent.label}</span>
                </button>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
