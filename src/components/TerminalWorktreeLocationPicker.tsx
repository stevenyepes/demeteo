import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react';
import { Check, ChevronDown, FolderGit2, GitBranch, House, Plus, RotateCw } from 'lucide-react';

import type { TerminalBranchOption, TerminalWorktree } from '../types';
import {
  createTerminalWorktree,
  listTerminalBranches,
  listTerminalLocations,
  removeTerminalWorktree,
} from '../lib/terminal';
import { formatError } from '../lib/errors';
import { CreateWorktreeForm, type WorktreeDraft } from './worktree/CreateWorktreeForm';
import { WorktreeRow } from './worktree/WorktreeRow';

/** A terminal target selected by a user. Worktree paths always come from the
 * backend; `main` deliberately leaves directory resolution to the launcher,
 * and `home` carries no repository scope at all — a launcher receiving it must
 * open at the machine's own root rather than anywhere inside the project.
 *
 * `main` carries a null `workBranch` on purpose, and the branch shown beside it
 * is a *report*, not a request: the main checkout is shared with anything else
 * using this project, so a session opening there takes the branch it finds
 * rather than checking one out under whoever else is working in it. */
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
  /** Both launchers raise this for the duration of a launch, so its rising
   *  edge doubles as the one HEAD-moving event this picker can observe. */
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
 * Shared location controller for terminal launchers. It owns only selection,
 * typed worktree discovery, creation, and retirement; callers own terminal
 * opening.
 *
 * Presented as a *field* rather than a button, because it is not an action —
 * it is the "where" half of a launch whose "what" half is the caller's own
 * button. Two adjacent buttons of equal weight read as two ways to start a
 * session, and the pair was mistaken for exactly that.
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
  const [mainBranch, setMainBranch] = useState<string | null>(null);
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  const [selected, setSelected] = useState<TerminalWorktreeLocation | null>(
    requireSelection ? null : MAIN_LOCATION,
  );
  const [creatingOpen, setCreatingOpen] = useState(false);
  const [branches, setBranches] = useState<TerminalBranchOption[]>([]);
  const [defaultBranch, setDefaultBranch] = useState('main');
  const [loadingBranches, setLoadingBranches] = useState(false);
  const [listing, setListing] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const createInFlight = useRef(false);
  // A request identity prevents a previous target's completion from clearing
  // the busy state of a newer create after the picker is retargeted.
  const activeCreateRequest = useRef<symbol | null>(null);

  const targetKey = `${projectId}:${repositoryId}`;
  const currentTargetKey = useRef(targetKey);
  currentTargetKey.current = targetKey;
  const busy = disabled || listing || creating;

  // The reset below wipes the create form, so it must key on the target alone.
  // A caller passing an inline `onChange` would otherwise clear a half-typed
  // branch name on every parent re-render.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const requireSelectionRef = useRef(requireSelection);
  requireSelectionRef.current = requireSelection;
  const selectedRef = useRef(selected);
  selectedRef.current = selected;

  useEffect(() => {
    onBusyChange?.(listing || creating);
  }, [listing, creating, onBusyChange]);

  // The annotation on the closed field outlives the read that produced it, and
  // the session this picker just launched into the main checkout is free to
  // check something else out — nothing reports that back. A launch is
  // therefore where the branch stops being vouchable, so it is dropped rather
  // than left asserting the name of a branch the shell has since left. The
  // next open re-reads it; "Main checkout" alone is never wrong.
  const wasDisabled = useRef(disabled);
  useEffect(() => {
    if (disabled && !wasDisabled.current) setMainBranch(null);
    wasDisabled.current = disabled;
  }, [disabled]);

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
    setMainBranch(null);
    setLoadedFor(null);
    setSelected(next);
    setCreatingOpen(false);
    setBranches([]);
    setError(null);
    setNotice(null);
    if (next) onChangeRef.current(next);
  }, [targetKey]);

  const load = useCallback(async () => {
    if (!projectId || !repositoryId || listing || creating) return;
    setListing(true);
    setError(null);
    const requestedTarget = `${projectId}:${repositoryId}`;
    try {
      const result = await listTerminalLocations(projectId, repositoryId);
      // Changes to props may race the request. Its eventual response is stale.
      if (requestedTarget === currentTargetKey.current) {
        setWorktrees(result.worktrees);
        setMainBranch(result.mainBranch);
        setLoadedFor(requestedTarget);
      }
    } catch (err) {
      if (requestedTarget === currentTargetKey.current) {
        setError(formatError(err));
        // A checkout that could not be read is exactly the case the `null`
        // contract covers, so the previous answer must not survive the failure
        // that replaced it — it would keep naming a branch for a directory the
        // backend just refused to reach.
        setMainBranch(null);
      }
    } finally {
      if (requestedTarget === currentTargetKey.current) setListing(false);
    }
  }, [projectId, repositoryId, listing, creating, targetKey]);

  // Only when the create form opens. Reading refs is cheap but not free on a
  // remote host, and every other use of this menu is a selection.
  const loadBranches = useCallback(async () => {
    if (!projectId || !repositoryId) return;
    setLoadingBranches(true);
    const requestedTarget = `${projectId}:${repositoryId}`;
    try {
      const options = await listTerminalBranches(projectId, repositoryId);
      if (requestedTarget === currentTargetKey.current) {
        setBranches(options.branches);
        setDefaultBranch(options.defaultBranch);
      }
    } catch (err) {
      // A base can still be typed-through from the default; failing to list
      // refs must not block creation, so this is reported and not fatal.
      if (requestedTarget === currentTargetKey.current) setError(formatError(err));
    } finally {
      if (requestedTarget === currentTargetKey.current) setLoadingBranches(false);
    }
  }, [projectId, repositoryId, targetKey]);

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
  // `list_terminal_locations` per open.
  //
  // Every open refetches. Worktrees appear and disappear while this menu is
  // closed — a pipeline finishing, another window, a `git worktree` in a
  // terminal — so a list cached from the first open goes stale in place, and
  // choosing a vanished one opens a session on a path that no longer exists.
  const toggleMenu = useCallback(() => {
    if (busy) return;
    const willOpen = !menuOpen;
    setMenuOpen(willOpen);
    setNotice(null);
    if (willOpen) void load();
  }, [busy, menuOpen, load]);

  const openCreate = useCallback(() => {
    setCreatingOpen(true);
    setError(null);
    setNotice(null);
    void loadBranches();
  }, [loadBranches]);

  const create = useCallback(
    async (draft: WorktreeDraft) => {
      // State does not update synchronously, so retain an imperative latch for
      // rapid double clicks that happen before the disabled attribute commits.
      if (createInFlight.current || busy) return;
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
          branch: draft.branch,
          baseBranch: draft.baseBranch,
          worktreeName: draft.worktreeName,
        });
        if (requestedTarget !== currentTargetKey.current || activeCreateRequest.current !== request)
          return;
        setWorktrees((previous) => [
          ...previous.filter((item) => item.path !== created.worktree.path),
          created.worktree,
        ]);
        setLoadedFor(requestedTarget);
        setCreatingOpen(false);
        // What it was actually cut from, not what was requested: the backend
        // falls back to a local ref when origin is unreachable, and that is
        // precisely when a caller assuming otherwise would be wrong.
        setNotice(`${draft.branch} · from ${created.baseRef}`);
        setSelected(worktreeLocation(created.worktree));
        onChange(worktreeLocation(created.worktree));
      } catch (err) {
        if (
          requestedTarget === currentTargetKey.current &&
          activeCreateRequest.current === request
        ) {
          setError(formatError(err));
        }
      } finally {
        if (
          requestedTarget === currentTargetKey.current &&
          activeCreateRequest.current === request
        ) {
          activeCreateRequest.current = null;
          createInFlight.current = false;
          setCreating(false);
        }
      }
    },
    [busy, projectId, repositoryId, targetKey, onChange],
  );

  const remove = useCallback(
    async (worktree: TerminalWorktree, force: boolean) => {
      await removeTerminalWorktree(projectId, repositoryId, worktree.path, force);
      if (targetKey !== currentTargetKey.current) return;
      setWorktrees((previous) => previous.filter((item) => item.path !== worktree.path));
      setNotice(null);
      // A session cannot be launched into a directory that no longer exists,
      // so a selection pointing at it has to go back to the main checkout.
      // Decided outside the state updater, which StrictMode invokes twice.
      if (selectedRef.current?.kind === 'worktree' && selectedRef.current.workDir === worktree.path) {
        const next = requireSelectionRef.current ? null : MAIN_LOCATION;
        setSelected(next);
        onChangeRef.current(next ?? MAIN_LOCATION);
      }
    },
    [projectId, repositoryId, targetKey],
  );

  // Only the main checkout's label is conditional on a fetch: the worktrees
  // name their own branch, and this one is a directory whose branch nothing
  // here chose. Before the first open there is nothing truthful to add, so it
  // reads exactly as it did — never a placeholder branch.
  const mainLabel = mainBranch ? `Main checkout · ${mainBranch}` : 'Main checkout';
  const selectedLabel =
    selected === null
      ? 'Choose a location'
      : selected.kind === 'main'
        ? mainLabel
        : selected.kind === 'home'
          ? 'Machine home'
          : (selected.workBranch ?? selected.workDir);

  return (
    <div className={`relative ${className}`} data-testid="terminal-worktree-location-picker">
      <button
        type="button"
        onClick={toggleMenu}
        disabled={busy || !projectId || !repositoryId}
        aria-expanded={menuOpen}
        aria-haspopup="menu"
        className={`w-full flex items-center gap-2 rounded-lg border bg-black/20 px-2.5 py-2 text-left text-[11.5px] font-mono transition disabled:opacity-40 ${
          menuOpen
            ? 'border-violet-400/50 text-slate-100'
            : 'border-white/10 text-slate-300 hover:border-white/20 hover:bg-black/30'
        }`}
        data-testid="terminal-location-trigger"
      >
        {selected?.kind === 'worktree' ? (
          <GitBranch className="w-3.5 h-3.5 shrink-0 text-violet-400" />
        ) : (
          <House className="w-3.5 h-3.5 shrink-0 text-slate-400" />
        )}
        <span className="flex-1 truncate">{selectedLabel}</span>
        <ChevronDown
          className={`w-3.5 h-3.5 shrink-0 text-slate-500 transition-transform ${menuOpen ? 'rotate-180' : ''}`}
        />
      </button>

      {menuOpen && (
        <div
          role="menu"
          className="absolute left-0 mt-1.5 z-30 w-[320px] rounded-xl border border-white/10 bg-[#0c0d12]/95 backdrop-blur-xl p-1.5 shadow-2xl"
          data-testid="terminal-location-menu"
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => choose(MAIN_LOCATION)}
            disabled={busy}
            className="w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] font-mono text-slate-300 hover:bg-white/5 disabled:opacity-40"
            data-testid="terminal-location-main"
          >
            <FolderGit2 className="w-3 h-3 shrink-0 text-slate-400" />
            <span className="shrink-0">Main checkout</span>
            {/* Reported, not chosen — see `TerminalWorktreeLocation`. This is
                the branch the session inherits, and the only place the user
                can see it before the shell draws its first prompt. */}
            <span
              className="min-w-0 flex-1 truncate text-right text-slate-600"
              data-testid="terminal-location-main-branch"
            >
              {mainBranch ? (
                <>
                  on <span className="text-slate-400">{mainBranch}</span>
                </>
              ) : null}
            </span>
            {selected?.kind === 'main' && <Check className="w-3 h-3 shrink-0 text-cyan-400" />}
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
          <div className="flex items-center gap-2 px-2 pb-1">
            <span className="text-[9px] font-mono uppercase tracking-[0.16em] text-slate-600">
              Worktrees
            </span>
            <span className="flex-1" />
            <button
              type="button"
              onClick={() => void load()}
              disabled={busy}
              className="p-0.5 rounded text-slate-600 hover:text-slate-300 hover:bg-white/5 transition disabled:opacity-40"
              title="Refresh"
              aria-label="Refresh worktrees"
              data-testid="terminal-location-refresh"
            >
              <RotateCw className={`w-3 h-3 ${listing ? 'animate-spin' : ''}`} />
            </button>
          </div>

          {listing && (
            <div
              className="px-2 py-1.5 text-[11px] font-mono text-slate-500"
              data-testid="terminal-location-loading"
            >
              Loading locations…
            </div>
          )}
          {!listing &&
            worktrees.map((worktree) => (
              <WorktreeRow
                key={worktree.path}
                worktree={worktree}
                active={selected?.kind === 'worktree' && selected.workDir === worktree.path}
                disabled={busy}
                onSelect={() => choose(worktreeLocation(worktree))}
                onRemove={(force) => remove(worktree, force)}
              />
            ))}
          {!listing && loadedFor === targetKey && worktrees.length === 0 && !creatingOpen && (
            <div className="px-2 py-1.5 text-[11px] font-mono text-slate-600">
              None yet — a worktree gives a session its own branch and folder.
            </div>
          )}

          <div className="my-1.5 border-t border-white/[0.06]" />
          {creatingOpen ? (
            <CreateWorktreeForm
              branches={branches}
              defaultBranch={defaultBranch}
              loadingBranches={loadingBranches}
              busy={creating}
              error={error}
              onSubmit={(draft) => void create(draft)}
              onCancel={() => {
                setCreatingOpen(false);
                setError(null);
              }}
            />
          ) : (
            <>
              <button
                type="button"
                onClick={openCreate}
                disabled={busy}
                className="w-full flex items-center gap-2 rounded-md border border-dashed border-white/15 px-2 py-1.5 text-left text-[11px] font-mono text-slate-400 hover:border-violet-400/40 hover:text-violet-200 hover:bg-violet-500/[0.08] transition disabled:opacity-40"
                data-testid="terminal-location-new"
              >
                <Plus className="w-3 h-3" />
                <span>New worktree</span>
              </button>
              {notice && (
                <div
                  className="mt-1.5 flex items-center gap-1.5 rounded border border-emerald-500/25 bg-emerald-500/[0.08] px-2 py-1.5 text-[10.5px] font-mono text-emerald-300"
                  data-testid="terminal-location-notice"
                >
                  <Check className="w-3 h-3 shrink-0" />
                  <span className="truncate">{notice}</span>
                </div>
              )}
              {error && (
                <div
                  className="mt-1.5 rounded border border-ruby-500/30 bg-ruby-500/10 px-2 py-1.5 text-[11px] font-mono text-ruby-300"
                  data-testid="terminal-location-error"
                >
                  {error}
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
