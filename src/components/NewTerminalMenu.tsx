import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Plus,
  TerminalSquare,
  ChevronDown,
  ChevronRight,
  Sparkles,
  Search,
  GitBranch,
  House,
  Check,
} from 'lucide-react';

import type { Machine, Feature } from '../types';
import { useTerminalPanel } from '../hooks/useTerminalPanel';
import { useProject } from '../context/ProjectContext';
import { AGENTS, agentLabel, defaultAgentKinds, type AgentMeta } from '../lib/agents';
import {
  loadRecents,
  recordRecent,
  RECENTS_SHOWN,
  type TerminalRecent,
} from '../lib/terminalRecents';
import { getAgentConfigs, listMachines } from '../lib/machines';
import { fetchActiveFeatures, getFeatureWorktree } from '../lib/features';
import { MachineDot } from './ui/MachineDot';

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
 * The right pane also carries an "Open in" location switch: for the current
 * project's machine it lists the project's live feature branches, so a shell
 * or agent can start inside a feature's worktree checkout rather than the
 * machine's default directory. A feature's machine + path + branch resolve
 * lazily at launch via `feature_get_worktree`, which is also the guard that
 * the branch still exists — a resolution failure aborts the open.
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
  /** The one machine the current project's worktrees live on — the only
   *  machine that can host a feature checkout. */
  const projectMachineId = currentProject ? currentProject.remote_host || 'local' : null;

  const [openMenu, setOpenMenu] = useState(false);
  const [machines, setMachines] = useState<Machine[]>([]);
  const [localAgents, setLocalAgents] = useState<AgentMeta[]>(
    defaultAgentKinds().map((k) => AGENTS[k]),
  );
  const [launching, setLaunching] = useState(false);
  const [query, setQuery] = useState('');
  const [activeMachineId, setActiveMachineId] = useState('local');
  const [recents, setRecents] = useState<TerminalRecent[]>([]);
  const [features, setFeatures] = useState<Feature[]>([]);
  /** Selected feature checkout to launch into; null = machine default dir. */
  const [locationFeatureId, setLocationFeatureId] = useState<string | null>(null);
  const [locationMenuOpen, setLocationMenuOpen] = useState(false);

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
        const list = (await listMachines()) || [];
        if (!cancelled) setMachines(list.filter((m) => m.auth_type !== 'local'));
      } catch {
        if (!cancelled) setMachines([]);
      }
      try {
        const configs = (await getAgentConfigs('local')) || [];
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

  // Load the current project's live feature branches each time the menu opens
  // (active features change as pipelines run, so this stays fresh rather than
  // caching). Only active features are offered — they're the ones with a live
  // worktree; a merged/cleaned branch is never "active".
  useEffect(() => {
    if (!openMenu || !currentProjectId) {
      setFeatures([]);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const list = (await fetchActiveFeatures(currentProjectId)) || [];
        if (!cancelled) setFeatures(list);
      } catch {
        if (!cancelled) setFeatures([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [openMenu, currentProjectId]);

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

  // Feature checkouts are only openable on the machine their worktree lives
  // on — the current project's machine. On any other machine the "Open in"
  // switch is hidden and launches use the default directory.
  const showLocationSwitch =
    !!activeTarget && activeTarget.machineId === projectMachineId && features.length > 0;
  const selectedFeature = useMemo(
    () => features.find((f) => f.id === locationFeatureId) ?? null,
    [features, locationFeatureId],
  );

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
    setLocationFeatureId(null);
    setLocationMenuOpen(false);
  }, [openMenu]);

  // A feature checkout is bound to its machine, so moving off that machine
  // clears the selection — you can't check a branch out where it doesn't live.
  useEffect(() => {
    if (!showLocationSwitch) {
      setLocationFeatureId(null);
      setLocationMenuOpen(false);
    }
  }, [showLocationSwitch]);

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
    async (target: MachineTarget, agent?: AgentMeta, feature?: Feature | null) => {
      if (launching) return;
      setLaunching(true);
      setOpenMenu(false);
      try {
        if (feature) {
          // Resolve the worktree at launch time — the machine, path, and
          // branch all come from here, and a failure (branch/worktree gone)
          // aborts the open rather than dropping the user in the wrong dir.
          const info = await getFeatureWorktree(feature.id);
          await open({
            machineId: info.machine_id,
            machineLabel: target.machineLabel,
            projectId: currentProjectId ?? undefined,
            workDir: info.worktree_path,
            workBranch: info.branch,
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
      } finally {
        setLaunching(false);
      }
    },
    [launching, open, currentProjectId],
  );

  // Primary split-button action: a bare shell on the local host, no menu.
  const launchLocalShell = useCallback(() => {
    void launch({
      machineId: 'local',
      machineLabel: 'local',
      sublabel: 'this machine',
      local: true,
      agents: localAgents,
    });
  }, [launch, localAgents]);

  // Arrow keys move the highlighted machine within the rail; Enter from the
  // search field launches a shell on it (honoring the selected location).
  // Runtime rows are focusable buttons, so Tab reaches them and Enter/Space
  // activates natively.
  const onMenuKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' && e.target === searchRef.current && activeTarget) {
        e.preventDefault();
        void launch(activeTarget, undefined, showLocationSwitch ? selectedFeature : null);
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
    [filteredTargets, activeTarget, launch, selectedFeature, showLocationSwitch],
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
          title="More — pick a machine, agent, or feature checkout"
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

                  {/* "Open in" location switch — machine default dir, or one of
                      the current project's live feature checkouts. */}
                  {showLocationSwitch && (
                    <div className="mx-1 mb-1.5" data-testid="new-terminal-location">
                      <button
                        type="button"
                        onClick={() => setLocationMenuOpen((v) => !v)}
                        className="w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-[11px] font-mono border border-white/10 bg-white/[0.03] text-slate-300 hover:bg-white/[0.06] transition"
                        aria-expanded={locationMenuOpen}
                        title="Choose where the shell or agent starts"
                      >
                        <span className="text-[9px] uppercase tracking-wide text-slate-500 shrink-0">
                          Open in
                        </span>
                        {selectedFeature ? (
                          <GitBranch className="w-3 h-3 text-violet-400 shrink-0" />
                        ) : (
                          <House className="w-3 h-3 text-slate-400 shrink-0" />
                        )}
                        <span className="truncate flex-1 text-left">
                          {selectedFeature ? selectedFeature.title || 'Untitled feature' : 'Home directory'}
                        </span>
                        <ChevronDown
                          className={`w-3 h-3 opacity-70 shrink-0 transition-transform ${
                            locationMenuOpen ? 'rotate-180' : ''
                          }`}
                        />
                      </button>

                      {locationMenuOpen && (
                        <div className="mt-1 rounded-md border border-white/10 bg-[#0e0f15] p-1 max-h-[168px] overflow-y-auto">
                          <button
                            type="button"
                            onClick={() => {
                              setLocationFeatureId(null);
                              setLocationMenuOpen(false);
                            }}
                            className="w-full flex items-center gap-2 rounded px-2 py-1.5 text-[11px] font-mono text-left text-slate-300 hover:bg-white/5 transition"
                          >
                            <House className="w-3 h-3 text-slate-400 shrink-0" />
                            <span className="flex-1">Home directory</span>
                            {!selectedFeature && <Check className="w-3 h-3 text-cyan-400 shrink-0" />}
                          </button>
                          <div className="my-1 mx-1 border-t border-white/[0.06]" />
                          <div className="px-2 pb-1 text-[8.5px] font-mono uppercase tracking-[0.16em] text-slate-600">
                            Feature checkouts
                          </div>
                          {features.map((f) => (
                            <button
                              key={f.id}
                              type="button"
                              onClick={() => {
                                setLocationFeatureId(f.id);
                                setLocationMenuOpen(false);
                              }}
                              className="w-full flex items-center gap-2 rounded px-2 py-1.5 text-[11px] font-mono text-left text-slate-300 hover:bg-violet-500/10 hover:text-violet-200 transition"
                              title={`Start in ${f.title || 'this feature'}'s checkout`}
                            >
                              <GitBranch className="w-3 h-3 text-violet-400 shrink-0" />
                              <span className="flex-1 truncate">{f.title || 'Untitled feature'}</span>
                              {f.agent_kind && (
                                <span className="text-[9px] text-slate-500 shrink-0">
                                  {agentLabel(f.agent_kind)}
                                </span>
                              )}
                              {locationFeatureId === f.id && (
                                <Check className="w-3 h-3 text-cyan-400 shrink-0" />
                              )}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  )}

                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => void launch(activeTarget, undefined, selectedFeature)}
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
                      onClick={() => void launch(activeTarget, agent, selectedFeature)}
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
    </div>
  );
}
