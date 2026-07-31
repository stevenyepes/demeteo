import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Plus,
  TerminalSquare,
  ChevronDown,
  ChevronRight,
  Sparkles,
  Search,
} from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';

import type { Machine } from '../types';
import { useTerminalPanel } from '../hooks/useTerminalPanel';
import { useProject } from '../context/ProjectContext';
import { AGENTS, agentLabel, defaultAgentKinds, type AgentMeta } from '../lib/agents';
import {
  loadRecents,
  recordRecent,
  RECENTS_SHOWN,
  type TerminalRecent,
} from '../lib/terminalRecents';
import { formatError } from '../lib/errors';
import { MachineDot } from './ui/MachineDot';
import {
  TerminalWorktreeLocationPicker,
  type TerminalWorktreeLocation,
} from './TerminalWorktreeLocationPicker';

/** One openable target: a machine plus the coding agents it offers. */
interface MachineTarget {
  machineId: string;
  machineLabel: string;
  /** Secondary line (host) shown for remotes; empty for local. */
  sublabel: string;
  local: boolean;
  agents: AgentMeta[];
}

/** Resolve the enabled, known agents for a machine from its stored `agents`
 *  JSON (an array of kinds), falling back to the sensible defaults. */
function agentsFromMachine(agentsJson: string | null | undefined): AgentMeta[] {
  let kinds: string[] = [];
  if (agentsJson) {
    try {
      const parsed = JSON.parse(agentsJson);
      if (Array.isArray(parsed)) kinds = parsed.filter((k) => typeof k === 'string');
    } catch {
      kinds = [];
    }
  }
  const known = kinds.filter((k) => AGENTS[k]);
  const chosen = known.length > 0 ? known : defaultAgentKinds();
  return chosen.map((k) => AGENTS[k]).filter(Boolean);
}

/** Does a machine match the free-text filter? Matches on name, host line, and
 *  the labels of the agents it offers, so typing `prod claude` or `codex`
 *  narrows the rail. */
function matchesQuery(target: MachineTarget, query: string): boolean {
  if (!query) return true;
  const hay = [
    target.machineLabel,
    target.sublabel,
    ...target.agents.map((a) => a.label),
  ]
    .join(' ')
    .toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((term) => hay.includes(term));
}

export interface NewTerminalMenuProps {
  className?: string;
  /** Tight placements (the `w-56` session-list header) shorten the primary
   *  label to `New` so the split button doesn't wrap; roomy placements (the
   *  empty state) keep the fuller `New shell`. */
  compact?: boolean;
  /** Monotonic counter that opens the launcher when it increments — lets a
   *  keyboard shortcut (Cmd/Ctrl+T) pop the menu from outside the component.
   *  The initial value never opens it; only a change past mount does. */
  openSignal?: number;
}

/**
 * The `+ New` launcher — opens a fresh terminal on any known machine (local
 * or a configured remote), as a bare shell or with a coding agent launched
 * straight into it (spec §5, §7).
 *
 * A flat list of every machine × runtime doesn't scale past a couple of
 * machines, so the launcher is structured around the two decisions a launch
 * actually is — *where* and *what*:
 *
 *   • a split button whose primary click opens a local shell in one step;
 *   • a "Recent" strip of the last machine × runtime launches (one-tap
 *     re-open — the common case at any machine count);
 *   • a two-pane machine → runtime picker (rail scrolls machines, right pane
 *     lists that machine's runtimes once) with a search field on top.
 *
 * The right pane also carries the shared terminal worktree picker for the
 * current project's repository. It exposes the primary checkout, existing
 * linked worktrees, and creation through the same typed API as Project Home.
 *
 * Every open uses `forceNew` so the launcher can stack multiple sessions on
 * the same machine. Absorbs the agent-config loading the retired
 * `AgentTerminalDrawer` owned (finding F4).
 */
export function NewTerminalMenu({
  className = '',
  compact = false,
  openSignal,
}: NewTerminalMenuProps): React.ReactElement {
  const { open } = useTerminalPanel();
  const { state: projectState } = useProject();
  const currentProjectId = projectState.currentProjectId;
  const currentProject = useMemo(
    () => projectState.projects.find((p) => p.id === currentProjectId) ?? null,
    [projectState.projects, currentProjectId],
  );
  /** The one machine the current project's repositories live on. */
  const projectMachineId = currentProject ? currentProject.remote_host || 'local' : null;
  const projectRepository = currentProjectId
    ? projectState.reposByProject[currentProjectId]?.[0] ?? null
    : null;

  const [openMenu, setOpenMenu] = useState(false);
  const [machines, setMachines] = useState<Machine[]>([]);
  const [localAgents, setLocalAgents] = useState<AgentMeta[]>(
    defaultAgentKinds().map((k) => AGENTS[k]),
  );
  const [launching, setLaunching] = useState(false);
  const [query, setQuery] = useState('');
  const [activeMachineId, setActiveMachineId] = useState('local');
  const [recents, setRecents] = useState<TerminalRecent[]>([]);
  const [location, setLocation] = useState<TerminalWorktreeLocation | null>(null);
  const [locationBusy, setLocationBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const launchingRef = useRef(false);
  const locationTargetRef = useRef(`${currentProjectId ?? ''}:${projectRepository?.id ?? ''}`);

  const containerRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const searchRef = useRef<HTMLInputElement | null>(null);

  // Load the machine list + the local machine's agent config once the menu
  // is first opened (cheap, and avoids the IPC on every panel mount). We
  // keep results cached across re-opens.
  const loadedRef = useRef(false);
  useEffect(() => {
    if (!openMenu || loadedRef.current) return;
    loadedRef.current = true;
    let cancelled = false;
    void (async () => {
      try {
        const list = (await invoke<Machine[]>('get_machines')) || [];
        if (!cancelled) setMachines(list.filter((m) => m.auth_type !== 'local'));
      } catch {
        if (!cancelled) setMachines([]);
      }
      try {
        const configs =
          (await invoke<Array<{ kind: string; enabled: boolean }>>('get_agent_configs', {
            machineId: 'local',
          })) || [];
        const found = configs
          .filter((c) => c.enabled && AGENTS[c.kind])
          .map((c) => AGENTS[c.kind]);
        if (!cancelled && found.length > 0) setLocalAgents(found);
      } catch {
        /* keep defaults */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [openMenu]);

  const targets = useMemo<MachineTarget[]>(() => {
    const local: MachineTarget = {
      machineId: 'local',
      machineLabel: 'local',
      sublabel: 'this machine',
      local: true,
      agents: localAgents,
    };
    const remotes: MachineTarget[] = machines.map((m) => ({
      machineId: m.id,
      machineLabel: m.name || m.host,
      sublabel: m.username ? `${m.username}@${m.host}` : m.host,
      local: false,
      agents: agentsFromMachine(m.agents),
    }));
    return [local, ...remotes];
  }, [machines, localAgents]);

  const filteredTargets = useMemo(
    () => targets.filter((t) => matchesQuery(t, query)),
    [targets, query],
  );

  // The rail's highlighted machine, always resolved to something present in
  // the current filter — so narrowing the list can't leave the actions pane
  // pointing at a machine that scrolled out of view.
  const activeTarget = useMemo(
    () =>
      filteredTargets.find((t) => t.machineId === activeMachineId) ??
      filteredTargets[0] ??
      null,
    [filteredTargets, activeMachineId],
  );

  // Project repository worktrees are only openable on their project's machine.
  const showLocationSwitch =
    !!activeTarget && activeTarget.machineId === projectMachineId && !!projectRepository;
  // The split button always targets local. When local is the current
  // project's machine, it must enter the same explicit location flow as the
  // menu rather than silently opening the primary checkout.
  const primaryRequiresLocationSelection =
    projectMachineId === 'local' && !!projectRepository && !!currentProjectId;

  // Recent launches resolved against the *live* machine list: drop any whose
  // machine no longer exists, and refresh the label so a rename shows through.
  const resolvedRecents = useMemo(() => {
    const byId = new Map(targets.map((t) => [t.machineId, t] as const));
    return recents
      .map((r) => {
        const target = byId.get(r.machineId);
        if (!target) return null;
        return { recent: r, target };
      })
      .filter((x): x is { recent: TerminalRecent; target: MachineTarget } => x !== null)
      .slice(0, RECENTS_SHOWN);
  }, [recents, targets]);

  // An external open request (Cmd/Ctrl+T). Skip the mount value so the menu
  // doesn't spring open on first render; only a later increment opens it.
  const openSignalSeen = useRef(openSignal);
  useEffect(() => {
    if (openSignal === undefined || openSignal === openSignalSeen.current) return;
    openSignalSeen.current = openSignal;
    setOpenMenu(true);
  }, [openSignal]);

  // On open: load the recents strip and reset the transient picker state.
  useEffect(() => {
    if (!openMenu) return;
    setRecents(loadRecents());
    setQuery('');
    setActiveMachineId('local');
    setError(null);
  }, [openMenu]);

  // A project checkout is bound to its machine, so moving off that machine
  // clears the selection — it must never be opened on another host.
  useEffect(() => {
    if (!showLocationSwitch) {
      setLocation(null);
      setError(null);
    }
  }, [showLocationSwitch]);

  // A launch failure belongs to one repository selection. Do not leave it
  // visible after the current project changes while remaining on local (or
  // after a repository refresh replaces the selected checkout).
  useEffect(() => {
    const target = `${currentProjectId ?? ''}:${projectRepository?.id ?? ''}`;
    if (locationTargetRef.current === target) return;
    locationTargetRef.current = target;
    setLocation(null);
    setError(null);
  }, [currentProjectId, projectRepository?.id]);

  // While open: close on an outside click or Escape, and move focus into the
  // search field so keyboard users can type-to-filter straight away.
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
    // Defer focus a tick so the popover has painted.
    const t = window.setTimeout(() => searchRef.current?.focus(), 0);
    return () => {
      document.removeEventListener('mousedown', onDocClick);
      document.removeEventListener('keydown', onKeyDown);
      window.clearTimeout(t);
    };
  }, [openMenu]);

  const launch = useCallback(
    async (target: MachineTarget, agent?: AgentMeta) => {
      if (launchingRef.current || locationBusy) return;
      // A project checkout is never an implicit target. This also protects
      // Enter from the search field if the disabled action has not yet painted.
      if (showLocationSwitch && !location) return;
      launchingRef.current = true;
      setLaunching(true);
      setOpenMenu(false);
      setError(null);
      try {
        if (showLocationSwitch && location && projectRepository && currentProjectId) {
          await open({
            machineId: target.machineId,
            machineLabel: target.machineLabel,
            projectId: currentProjectId,
            repoPath: projectRepository.repo_path,
            workDir: location.workDir ?? undefined,
            workBranch: location.workBranch,
            forceNew: true,
            launchCommand: agent?.binary,
            agentKind: agent?.kind ?? null,
          });
        } else {
          // Plain machine-root launches feed the Recent strip; feature-scoped
          // ones don't (a re-clicked chip couldn't faithfully reopen a
          // possibly-gone checkout).
          setRecents(
            recordRecent({
              machineId: target.machineId,
              machineLabel: target.machineLabel,
              agentKind: agent?.kind ?? null,
            }),
          );
          await open({
            machineId: target.machineId,
            machineLabel: target.machineLabel,
            forceNew: true,
            launchCommand: agent?.binary,
            agentKind: agent?.kind ?? null,
          });
        }
      } catch (err) {
        console.warn('[NewTerminalMenu] open failed:', err);
        setError(formatError(err));
      } finally {
        launchingRef.current = false;
        setLaunching(false);
      }
    },
    [locationBusy, showLocationSwitch, location, projectRepository, currentProjectId, open],
  );

  // Primary split-button action: preserve one-click machine-root launches,
  // except when local is the active project's machine. That checkout needs a
  // deliberate main/worktree choice, so reveal the shared chooser instead.
  const launchLocalShell = useCallback(() => {
    if (primaryRequiresLocationSelection) {
      setOpenMenu(true);
      return;
    }
    void launch({
      machineId: 'local',
      machineLabel: 'local',
      sublabel: 'this machine',
      local: true,
      agents: localAgents,
    });
  }, [launch, localAgents, primaryRequiresLocationSelection]);

  // Arrow keys move the highlighted machine within the rail; Enter from the
  // search field launches a shell on it (honoring the selected location).
  // Runtime rows are focusable buttons, so Tab reaches them and Enter/Space
  // activates natively.
  const onMenuKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' && e.target === searchRef.current && activeTarget) {
        e.preventDefault();
        void launch(activeTarget);
        return;
      }
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
      if (filteredTargets.length === 0) return;
      e.preventDefault();
      const idx = filteredTargets.findIndex((t) => t.machineId === activeTarget?.machineId);
      const delta = e.key === 'ArrowDown' ? 1 : -1;
      const nextIdx = Math.min(
        filteredTargets.length - 1,
        Math.max(0, (idx < 0 ? 0 : idx) + delta),
      );
      setActiveMachineId(filteredTargets[nextIdx].machineId);
    },
    [filteredTargets, activeTarget, launch],
  );

  return (
    <div ref={containerRef} className={`relative ${className}`} data-testid="new-terminal-menu">
      {/* Split button: primary opens a local shell; the caret opens the picker. */}
      <div className="inline-flex shrink-0 rounded-md shadow-sm">
        <button
          ref={triggerRef}
          type="button"
          onClick={launchLocalShell}
          disabled={launching}
          className="flex items-center gap-1.5 pl-2 pr-2 py-1 rounded-l-md text-[11px] font-mono whitespace-nowrap border border-white/10 border-r-0 bg-white/5 text-slate-300 hover:bg-cyan-500/15 hover:border-cyan-500/40 hover:text-cyan-300 transition disabled:opacity-40"
          title="Open a shell on this machine"
          data-testid="new-terminal-trigger"
        >
          <Plus className="w-3.5 h-3.5 shrink-0" />
          <span>{compact ? 'New' : 'New shell'}</span>
        </button>
        <button
          type="button"
          onClick={() => setOpenMenu((v) => !v)}
          disabled={launching}
          className="flex items-center px-1.5 py-1 rounded-r-md text-[11px] font-mono border border-white/10 bg-white/5 text-slate-300 hover:bg-cyan-500/15 hover:border-cyan-500/40 hover:text-cyan-300 transition disabled:opacity-40"
          title="More — pick a machine, agent, or terminal location"
          aria-haspopup="menu"
          aria-expanded={openMenu}
          data-testid="new-terminal-caret"
        >
          <ChevronDown className="w-3 h-3 opacity-70" />
        </button>
      </div>

      {openMenu &&
        (() => {
          const panel = (
            <div
              ref={menuRef}
              role="menu"
              onKeyDown={onMenuKeyDown}
              className={
                compact
                  ? 'absolute left-0 mt-1 z-30 w-[440px] max-w-[92vw] rounded-lg border border-white/10 bg-[#0c0d12] shadow-xl overflow-hidden'
                  : 'relative z-50 w-[440px] max-w-[92vw] rounded-lg border border-white/10 bg-[#0c0d12] shadow-2xl overflow-hidden'
              }
              data-testid="new-terminal-dropdown"
            >
          {/* Search */}
          <div className="flex items-center gap-2.5 px-3 py-2.5 border-b border-white/[0.07]">
            <Search className="w-3.5 h-3.5 text-slate-500 shrink-0" />
            <input
              ref={searchRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search machines & agents…"
              autoComplete="off"
              spellCheck={false}
              className="flex-1 bg-transparent border-0 outline-none text-[12px] font-mono text-slate-200 placeholder:text-slate-600"
              data-testid="new-terminal-search"
            />
            <span className="text-[9px] font-mono text-slate-600 border border-white/10 rounded px-1 py-px">
              esc
            </span>
          </div>

          {/* Recent strip */}
          {resolvedRecents.length > 0 && query === '' && (
            <div className="border-b border-white/[0.06]">
              <div className="px-3 pt-2.5 pb-1 text-[9px] font-mono uppercase tracking-[0.16em] text-slate-600">
                Recent
              </div>
              <div className="flex flex-wrap gap-1.5 px-3 pb-2.5">
                {resolvedRecents.map(({ recent, target }) => {
                  const label = agentLabel(recent.agentKind);
                  const agent = recent.agentKind
                    ? target.agents.find((a) => a.kind === recent.agentKind) ??
                      AGENTS[recent.agentKind]
                    : undefined;
                  return (
                    <button
                      key={`${recent.machineId}:${recent.agentKind ?? 'shell'}`}
                      type="button"
                      role="menuitem"
                      onClick={() => void launch(target, agent)}
                      className="inline-flex items-center gap-1.5 rounded-full border border-white/10 bg-white/[0.03] px-2.5 py-1 text-[11px] font-mono text-slate-300 hover:bg-violet-500/15 hover:border-violet-400/40 hover:text-violet-200 transition"
                      title={`Open ${label ?? 'shell'} on ${target.machineLabel}`}
                    >
                      <MachineDot
                        machineId={target.machineId}
                        machineLabel={target.machineLabel}
                      />
                      <span className="truncate max-w-[110px]">{target.machineLabel}</span>
                      <span className="text-slate-600">·</span>
                      <span className={label ? 'text-violet-300' : 'text-cyan-300'}>
                        {label ?? 'shell'}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {/* Two-pane machine → runtime picker */}
          <div className="grid grid-cols-[168px_1fr] min-h-[188px]">
            {/* Machine rail */}
            <div
              className="border-r border-white/[0.06] p-1.5 max-h-[300px] overflow-y-auto"
              data-testid="new-terminal-rail"
            >
              {filteredTargets.length === 0 && (
                <div className="px-2 py-6 text-center text-[11px] font-mono text-slate-600">
                  No machines match
                </div>
              )}
              {filteredTargets.map((target) => {
                const active = target.machineId === activeTarget?.machineId;
                return (
                  <button
                    key={target.machineId}
                    type="button"
                    onMouseEnter={() => setActiveMachineId(target.machineId)}
                    onFocus={() => setActiveMachineId(target.machineId)}
                    onClick={() => setActiveMachineId(target.machineId)}
                    className={`group relative w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11.5px] font-mono transition ${
                      active
                        ? 'bg-cyan-500/[0.12] text-slate-100'
                        : 'text-slate-400 hover:bg-white/[0.04]'
                    }`}
                    data-testid={`new-terminal-machine-${target.machineId}`}
                    aria-current={active}
                  >
                    {active && (
                      <span className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full bg-cyan-400" />
                    )}
                    <MachineDot
                      machineId={target.machineId}
                      machineLabel={target.machineLabel}
                    />
                    <span className="truncate flex-1">{target.machineLabel}</span>
                    <ChevronRight className="w-3 h-3 text-slate-600 opacity-0 group-hover:opacity-100" />
                  </button>
                );
              })}
            </div>

            {/* Runtimes for the highlighted machine */}
            <div className="p-2 max-h-[300px] overflow-y-auto" data-testid="new-terminal-actions">
              {activeTarget ? (
                <>
                  <div className="flex items-baseline gap-2 px-2 pt-1 pb-1.5">
                    <span className="text-[12px] font-mono font-semibold text-slate-200 truncate">
                      {activeTarget.machineLabel}
                    </span>
                    <span className="text-[9px] font-mono text-slate-600 truncate">
                      {activeTarget.sublabel}
                    </span>
                  </div>

                  {/* The global launcher shares Project Home's typed terminal
                      location selection instead of maintaining feature-only
                      worktree state. */}
                  {showLocationSwitch && (
                    <div className="mx-1 mb-1.5" data-testid="new-terminal-location">
                      <TerminalWorktreeLocationPicker
                        projectId={currentProjectId ?? ''}
                        repositoryId={projectRepository?.id ?? ''}
                        onChange={setLocation}
                        requireSelection
                        onBusyChange={setLocationBusy}
                        disabled={launching}
                      />
                    </div>
                  )}

                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => void launch(activeTarget)}
                    disabled={launching || locationBusy || (showLocationSwitch && !location)}
                    className="w-full flex items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-[12px] font-mono text-slate-300 hover:bg-cyan-500/15 hover:text-cyan-200 transition"
                    title={`Open a shell on ${activeTarget.machineLabel}`}
                  >
                    <TerminalSquare className="w-3.5 h-3.5 text-cyan-400 shrink-0" />
                    <span>New shell</span>
                    <span className="ml-auto text-[9px] font-mono text-slate-600 border border-white/10 rounded px-1">
                      ⏎
                    </span>
                  </button>

                  {activeTarget.agents.length > 0 && (
                    <div className="my-1.5 mx-2 border-t border-white/[0.06]" />
                  )}

                  {activeTarget.agents.map((agent) => (
                    <button
                      key={agent.kind}
                      type="button"
                      role="menuitem"
                      onClick={() => void launch(activeTarget, agent)}
                      disabled={launching || locationBusy || (showLocationSwitch && !location)}
                      className="w-full flex items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-[12px] font-mono text-slate-300 hover:bg-violet-500/15 hover:text-violet-200 transition"
                      title={`Run ${agent.binary} in a new terminal on ${activeTarget.machineLabel}`}
                    >
                      <Sparkles className="w-3.5 h-3.5 text-violet-400 shrink-0" />
                      <span>{agent.label}</span>
                    </button>
                  ))}
                </>
              ) : (
                <div className="px-2 py-6 text-center text-[11px] font-mono text-slate-600">
                  Nothing to launch
                </div>
              )}
            </div>
          </div>

          {/* Keyboard hint footer */}
          <div className="flex items-center gap-3 px-3 py-1.5 border-t border-white/[0.07] text-[9px] font-mono text-slate-600">
            <span>
              <kbd className="border border-white/10 rounded px-1 text-slate-500">↑↓</kbd> machine
            </span>
            <span>
              <kbd className="border border-white/10 rounded px-1 text-slate-500">⇥</kbd> runtime
            </span>
            <span>
              <kbd className="border border-white/10 rounded px-1 text-slate-500">⏎</kbd> launch
            </span>
          </div>
            </div>
          );

          // Anchored dropdown for the tight header placement; a screen-centered
          // modal (with a dimmed backdrop) for the roomy empty-state placement,
          // where hanging the popover off a centered button looks stranded.
          if (compact) return panel;
          return (
            <div
              className="fixed inset-0 z-40 flex items-start justify-center pt-[14vh] bg-black/50 backdrop-blur-sm"
              onMouseDown={(e) => {
                if (e.target === e.currentTarget) setOpenMenu(false);
              }}
              data-testid="new-terminal-overlay"
            >
              {panel}
            </div>
          );
        })()}
      {error && (
        <div className="absolute left-0 top-full z-30 mt-1.5 max-w-xs rounded border border-ruby-500/30 bg-ruby-500/10 px-2 py-1.5 text-[11px] font-mono text-ruby-300" data-testid="new-terminal-error">
          {error}
        </div>
      )}
    </div>
  );
}
