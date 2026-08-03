import { useState, type ReactElement } from 'react';
import { Check, GitBranch, Trash2 } from 'lucide-react';

import type { TerminalWorktree } from '../../types';
import { formatError } from '../../lib/errors';

export interface WorktreeRowProps {
  worktree: TerminalWorktree;
  active: boolean;
  disabled: boolean;
  onSelect: () => void;
  /** Rejects with the backend's own refusal, which the row shows verbatim. */
  onRemove: (force: boolean) => Promise<void>;
}

/** The last path component — the folder the user named, not the whole path. */
function folderName(path: string): string {
  const parts = path.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/**
 * One selectable worktree, with its own removal.
 *
 * Removal lives on the row rather than behind a separate management screen
 * because the list *is* where a stale worktree is noticed — the moment someone
 * opens this menu and scrolls past four branches they finished last week is the
 * only moment they will ever want to clean them up.
 *
 * The confirm step is inline and the refusal is git's own. A worktree holding
 * uncommitted work fails the first attempt and offers force as a second,
 * separate decision, so nothing a user has not saved is discarded by one click.
 */
export function WorktreeRow({
  worktree,
  active,
  disabled,
  onSelect,
  onRemove,
}: WorktreeRowProps): ReactElement {
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const label = worktree.branch ?? folderName(worktree.path);
  const folder = folderName(worktree.path);

  const remove = async (force: boolean) => {
    setRemoving(true);
    setError(null);
    try {
      await onRemove(force);
      setConfirming(false);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setRemoving(false);
    }
  };

  if (confirming) {
    return (
      <div
        className="rounded-md border border-ruby-500/25 bg-ruby-500/[0.07] px-2 py-1.5"
        data-testid={`terminal-location-confirm-${worktree.path}`}
      >
        <div className="text-[11px] font-mono text-slate-300 truncate">
          Remove <span className="text-ruby-300">{folder}</span>?
        </div>
        <div className="text-[9.5px] font-mono text-slate-500 pt-0.5">
          {error ? 'The branch is kept either way.' : `Deletes the folder. The branch ${label} is kept.`}
        </div>
        {error && (
          <div
            className="mt-1 rounded border border-ruby-500/30 bg-ruby-500/10 px-1.5 py-1 text-[10px] font-mono text-ruby-300 break-words"
            data-testid={`terminal-location-remove-error-${worktree.path}`}
          >
            {error}
          </div>
        )}
        <div className="flex items-center gap-1.5 pt-1.5">
          <button
            type="button"
            onClick={() => {
              setConfirming(false);
              setError(null);
            }}
            disabled={removing}
            className="flex-1 rounded px-2 py-1 text-[10.5px] font-mono text-slate-400 border border-white/10 hover:bg-white/5 transition disabled:opacity-40"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void remove(error !== null)}
            disabled={removing}
            className="flex-1 rounded px-2 py-1 text-[10.5px] font-mono text-white bg-ruby-600 hover:bg-ruby-500 transition disabled:opacity-40"
            data-testid={`terminal-location-remove-confirm-${worktree.path}`}
          >
            {removing ? 'Removing…' : error ? 'Remove anyway' : 'Remove'}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="group flex items-center gap-1">
      <button
        type="button"
        role="menuitem"
        onClick={onSelect}
        disabled={disabled}
        className={`min-w-0 flex-1 flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] font-mono transition disabled:opacity-40 ${
          active ? 'bg-violet-500/[0.14] text-slate-100' : 'text-slate-300 hover:bg-violet-500/15'
        }`}
        data-testid={`terminal-location-worktree-${worktree.path}`}
      >
        <GitBranch className="w-3 h-3 text-violet-400 shrink-0" />
        <span className="flex-1 truncate">{label}</span>
        {worktree.isLocked && <span className="text-[9px] text-amber-300 shrink-0">locked</span>}
        {active && <Check className="w-3 h-3 text-cyan-400 shrink-0" />}
      </button>
      <button
        type="button"
        onClick={() => setConfirming(true)}
        disabled={disabled}
        className="shrink-0 p-1.5 rounded-md text-slate-600 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 hover:text-ruby-300 hover:bg-ruby-500/10 transition disabled:opacity-40"
        title={`Remove the ${folder} worktree`}
        aria-label={`Remove the ${folder} worktree`}
        data-testid={`terminal-location-remove-${worktree.path}`}
      >
        <Trash2 className="w-3 h-3" />
      </button>
    </div>
  );
}
