import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react';
import { Check, ChevronDown, GitBranch, House, Plus } from 'lucide-react';

import type { TerminalWorktree } from '../types';
import { createTerminalWorktree, listTerminalWorktrees } from '../lib/terminal';
import { formatError } from '../lib/errors';

/** A terminal target selected by a user. Worktree paths always come from the
 * backend; `main` deliberately leaves directory resolution to the launcher,
 * and `home` carries no repository scope at all — a launcher receiving it must
 * open at the machine's own root rather than anywhere inside the project. */
export type TerminalWorktreeLocation =
  | { kind: 'main'; workDir: null; workBranch: null }
  | { kind: 'home'; workDir: null; workBranch: null }
  | { kind: 'worktree'; workDir: string; workBranch: string | null };

export interface TerminalWorktreeLocationPickerProps {
  projectId: string;
  repositoryId: string;
  /** Called whenever the selected location changes, including reset to main. */
  onChange: (location: TerminalWorktreeLocation) => void;
  /** Require a deliberate menu choice instead of treating main as selected. */
  requireSelection?: boolean;
  /** Offer the unscoped machine root. Only a launcher that can open outside
   *  the repository may set this; a repo-scoped one would emit a target it
   *  cannot honour. */
  allowHome?: boolean;
  /** Reports list/create activity so a containing launcher can stay disabled. */
  onBusyChange?: (busy: boolean) => void;
  disabled?: boolean;
  className?: string;
}

const MAIN_LOCATION: TerminalWorktreeLocation = {
  kind: 'main',
  workDir: null,
  workBranch: null,
};

const HOME_LOCATION: TerminalWorktreeLocation = {
  kind: 'home',
  workDir: null,
  workBranch: null,
};

function worktreeLocation(worktree: TerminalWorktree): TerminalWorktreeLocation {
  return {
    kind: 'worktree',
    workDir: worktree.path,
    workBranch: worktree.branch,
  };
}

/**
 * Shared location controller for terminal launchers. It owns only selection
 * and typed worktree discovery/creation; callers own terminal opening.
 */
export function TerminalWorktreeLocationPicker({
  projectId,
  repositoryId,
  onChange,
  requireSelection = false,
  allowHome = false,
  onBusyChange,
  disabled = false,
  className = '',
}: TerminalWorktreeLocationPickerProps): ReactElement {
  const [menuOpen, setMenuOpen] = useState(false);
  const [worktrees, setWorktrees] = useState<TerminalWorktree[]>([]);
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  const [selected, setSelected] = useState<TerminalWorktreeLocation | null>(
    requireSelection ? null : MAIN_LOCATION,
  );
  const [branch, setBranch] = useState('');
  const [worktreeName, setWorktreeName] = useState('');
  const [listing, setListing] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const createInFlight = useRef(false);
  // A request identity prevents a previous target's completion from clearing
  // the busy state of a newer create after the picker is retargeted.
  const activeCreateRequest = useRef<symbol | null>(null);

  const targetKey = `${projectId}:${repositoryId}`;
  const currentTargetKey = useRef(targetKey);
  currentTargetKey.current = targetKey;
  const busy = disabled || listing || creating;

  // The reset below wipes typed input, so it must key on the target alone. A
  // caller passing an inline `onChange` would otherwise clear a half-typed
  // branch name on every parent re-render.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const requireSelectionRef = useRef(requireSelection);
  requireSelectionRef.current = requireSelection;

  useEffect(() => {
    onBusyChange?.(listing || creating);
  }, [listing, creating, onBusyChange]);

  // A response for an old project/repository must never populate the current
  // picker. Reset both selection and failures before a new target can launch.
  useEffect(() => {
    // Retargeting invalidates any create that was issued for the old
    // project/repository. Its completion must not affect this picker.
    activeCreateRequest.current = null;
    createInFlight.current = false;
    setCreating(false);
  }, [targetKey]);

  useEffect(() => {
    const next = requireSelectionRef.current ? null : MAIN_LOCATION;
    setMenuOpen(false);
    setWorktrees([]);
    setLoadedFor(null);
    setSelected(next);
    setBranch('');
    setWorktreeName('');
    setError(null);
    if (next) onChangeRef.current(next);
  }, [targetKey]);

  const load = useCallback(async () => {
    if (!projectId || !repositoryId || listing || creating) return;
    setListing(true);
    setError(null);
    const requestedTarget = `${projectId}:${repositoryId}`;
    try {
      const result = await listTerminalWorktrees(projectId, repositoryId);
      // Changes to props may race the request. Its eventual response is stale.
      if (requestedTarget === currentTargetKey.current) {
        setWorktrees(result);
        setLoadedFor(requestedTarget);
      }
    } catch (err) {
      if (requestedTarget === currentTargetKey.current) setError(formatError(err));
    } finally {
      if (requestedTarget === currentTargetKey.current) setListing(false);
    }
  }, [projectId, repositoryId, listing, creating, targetKey]);

  const choose = useCallback(
    (location: TerminalWorktreeLocation) => {
      setSelected(location);
      setError(null);
      setMenuOpen(false);
      onChange(location);
    },
    [onChange],
  );

  // The list request must stay outside the state updater: StrictMode invokes
  // an updater twice, so a request inside one issues two
  // `list_terminal_worktrees` per open.
  //
  // Every open refetches. Worktrees appear and disappear while this menu is
  // closed — a pipeline finishing, another window, a `git worktree` in a
  // terminal — so a list cached from the first open goes stale in place, and
  // choosing a vanished one opens a session on a path that no longer exists.
  const toggleMenu = useCallback(() => {
    if (busy) return;
    const willOpen = !menuOpen;
    setMenuOpen(willOpen);
    if (willOpen) void load();
  }, [busy, menuOpen, load]);

  const create = useCallback(async () => {
    // State does not update synchronously, so retain an imperative latch for
    // rapid double clicks that happen before the disabled attribute commits.
    if (createInFlight.current || !branch.trim() || !worktreeName.trim() || busy) return;
    const requestedTarget = targetKey;
    const request = Symbol(requestedTarget);
    createInFlight.current = true;
    activeCreateRequest.current = request;
    setCreating(true);
    setError(null);
    try {
      const created = await createTerminalWorktree({
        projectId,
        repositoryId,
        branch: branch.trim(),
        worktreeName: worktreeName.trim(),
      });
      if (requestedTarget !== currentTargetKey.current || activeCreateRequest.current !== request) return;
      setWorktrees((previous) => [...previous.filter((item) => item.path !== created.path), created]);
      setLoadedFor(requestedTarget);
      setBranch('');
      setWorktreeName('');
      choose(worktreeLocation(created));
    } catch (err) {
      if (requestedTarget === currentTargetKey.current && activeCreateRequest.current === request) {
        setError(formatError(err));
      }
    } finally {
      if (requestedTarget === currentTargetKey.current && activeCreateRequest.current === request) {
        activeCreateRequest.current = null;
        createInFlight.current = false;
        setCreating(false);
      }
    }
  }, [branch, worktreeName, busy, projectId, repositoryId, targetKey, choose]);

  const selectedLabel =
    selected === null
      ? 'Choose a location'
      : selected.kind === 'main'
      ? 'Main branch'
      : selected.kind === 'home'
      ? 'Machine home'
      : selected.workBranch ?? selected.workDir;

  return (
    <div className={`relative ${className}`} data-testid="terminal-worktree-location-picker">
      <button
        type="button"
        onClick={toggleMenu}
        disabled={busy || !projectId || !repositoryId}
        aria-expanded={menuOpen}
        aria-haspopup="menu"
        className="w-full flex items-center gap-2 rounded-md border border-white/10 bg-white/[0.03] px-2 py-1.5 text-left text-[11px] font-mono text-slate-300 hover:bg-white/[0.06] transition disabled:opacity-40"
        data-testid="terminal-location-trigger"
      >
        {selected?.kind === 'worktree' ? <GitBranch className="w-3 h-3 text-violet-400" /> : <House className="w-3 h-3 text-slate-400" />}
        <span className="flex-1 truncate">{selectedLabel}</span>
        <ChevronDown className={`w-3 h-3 text-slate-500 transition-transform ${menuOpen ? 'rotate-180' : ''}`} />
      </button>

      {menuOpen && (
        <div role="menu" className="absolute left-0 mt-1 z-30 w-72 rounded-lg border border-white/10 bg-[#0c0d12] p-1.5 shadow-2xl" data-testid="terminal-location-menu">
          <button
            type="button"
            role="menuitem"
            onClick={() => choose(MAIN_LOCATION)}
            disabled={busy}
            className="w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] font-mono text-slate-300 hover:bg-white/5 disabled:opacity-40"
            data-testid="terminal-location-main"
          >
            <House className="w-3 h-3 text-slate-400" />
            <span className="flex-1">Main branch</span>
            {selected?.kind === 'main' && <Check className="w-3 h-3 text-cyan-400" />}
          </button>

          {allowHome && (
            <button
              type="button"
              role="menuitem"
              onClick={() => choose(HOME_LOCATION)}
              disabled={busy}
              className="w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] font-mono text-slate-300 hover:bg-white/5 disabled:opacity-40"
              data-testid="terminal-location-home"
            >
              <House className="w-3 h-3 text-slate-400" />
              <span className="flex-1">Machine home</span>
              {selected?.kind === 'home' && <Check className="w-3 h-3 text-cyan-400" />}
            </button>
          )}

          <div className="my-1 border-t border-white/[0.06]" />
          <div className="px-2 pb-1 text-[9px] font-mono uppercase tracking-[0.16em] text-slate-600">Linked worktrees</div>
          {listing && <div className="px-2 py-1.5 text-[11px] font-mono text-slate-500" data-testid="terminal-location-loading">Loading locations…</div>}
          {!listing && worktrees.map((worktree) => {
            const location = worktreeLocation(worktree);
            const active = selected?.kind === 'worktree' && selected.workDir === worktree.path;
            return (
              <button key={worktree.path} type="button" role="menuitem" onClick={() => choose(location)} disabled={busy} className="w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] font-mono text-slate-300 hover:bg-violet-500/15 disabled:opacity-40" data-testid={`terminal-location-worktree-${worktree.path}`}>
                <GitBranch className="w-3 h-3 text-violet-400 shrink-0" />
                <span className="flex-1 truncate">{worktree.branch ?? worktree.path}</span>
                {worktree.isLocked && <span className="text-[9px] text-amber-300">locked</span>}
                {active && <Check className="w-3 h-3 text-cyan-400 shrink-0" />}
              </button>
            );
          })}
          {!listing && loadedFor === targetKey && worktrees.length === 0 && <div className="px-2 py-1.5 text-[11px] font-mono text-slate-600">No linked worktrees</div>}

          <div className="my-1.5 border-t border-white/[0.06]" />
          <div className="px-2 pb-1 text-[9px] font-mono uppercase tracking-[0.16em] text-slate-600">Create linked worktree</div>
          <div className="grid grid-cols-2 gap-1 px-1">
            <input value={branch} onChange={(event) => { setBranch(event.target.value); setError(null); }} disabled={busy} placeholder="Branch" aria-label="Branch name" className="min-w-0 rounded border border-white/10 bg-black/20 px-2 py-1.5 text-[11px] font-mono text-slate-200 outline-none focus:border-violet-400 disabled:opacity-40" />
            <input value={worktreeName} onChange={(event) => { setWorktreeName(event.target.value); setError(null); }} disabled={busy} placeholder="Folder name" aria-label="Worktree name" className="min-w-0 rounded border border-white/10 bg-black/20 px-2 py-1.5 text-[11px] font-mono text-slate-200 outline-none focus:border-violet-400 disabled:opacity-40" />
          </div>
          <button type="button" onClick={() => void create()} disabled={busy || !branch.trim() || !worktreeName.trim()} className="mt-1.5 w-full flex items-center justify-center gap-1.5 rounded-md bg-violet-600 px-2 py-1.5 text-[11px] font-mono text-white hover:bg-violet-500 disabled:opacity-40" data-testid="terminal-location-create">
            <Plus className="w-3 h-3" /> {creating ? 'Creating…' : 'Create worktree'}
          </button>
          {error && <div className="mt-1.5 rounded border border-ruby-500/30 bg-ruby-500/10 px-2 py-1.5 text-[11px] font-mono text-ruby-300" data-testid="terminal-location-error">{error}</div>}
        </div>
      )}
    </div>
  );
}
