import { useMemo, useState, type ReactElement } from 'react';
import { Check, Pencil, RefreshCw, X } from 'lucide-react';

import type { TerminalBranchOption } from '../../types';
import { deriveWorktreeName } from '../../lib/worktrees';

/** What the picker needs to ask the backend for a worktree. */
export interface WorktreeDraft {
  branch: string;
  baseBranch: string;
  worktreeName: string;
}

export interface CreateWorktreeFormProps {
  branches: TerminalBranchOption[];
  /** The branch this project integrates into — preselected as the base. */
  defaultBranch: string;
  loadingBranches: boolean;
  busy: boolean;
  /** A failure from the last submit, owned by the caller that made the call. */
  error: string | null;
  onSubmit: (draft: WorktreeDraft) => void;
  onCancel: () => void;
}

/**
 * The "new worktree" half of the location picker: a branch, the base it is cut
 * from, and the folder that base lands in.
 *
 * The base is a control rather than an assumption. Creation used to start at
 * whatever the main checkout was sitting on, which is invisible from here and
 * usually behind origin — so the one thing this form has to make legible is
 * *where the branch will start*, stated before the click and confirmed after
 * it. Everything else on screen is subordinate to that.
 */
export function CreateWorktreeForm({
  branches,
  defaultBranch,
  loadingBranches,
  busy,
  error,
  onSubmit,
  onCancel,
}: CreateWorktreeFormProps): ReactElement {
  const [branch, setBranch] = useState('');
  const [base, setBase] = useState<string | null>(null);
  // The folder follows the branch until someone types in it; after that it is
  // theirs, and a later branch edit must not overwrite what they chose.
  const [folderOverride, setFolderOverride] = useState<string | null>(null);

  const selectedBase = useMemo(() => {
    if (base !== null) return base;
    if (branches.some((option) => option.name === defaultBranch)) return defaultBranch;
    return branches[0]?.name ?? defaultBranch;
  }, [base, branches, defaultBranch]);

  const baseOption = branches.find((option) => option.name === selectedBase) ?? null;
  const derived = deriveWorktreeName(branch);
  const folder = folderOverride ?? derived;
  const canSubmit = !busy && branch.trim().length > 0 && folder.length > 0;

  return (
    <div className="px-1 pb-1" data-testid="terminal-location-create-form">
      <div className="flex items-center justify-between px-1 pb-1.5">
        <span className="text-[9px] font-mono uppercase tracking-[0.16em] text-slate-500">
          New worktree
        </span>
        <button
          type="button"
          onClick={onCancel}
          disabled={busy}
          className="p-0.5 rounded text-slate-500 hover:text-slate-200 hover:bg-white/5 transition disabled:opacity-40"
          title="Cancel"
          aria-label="Cancel new worktree"
          data-testid="terminal-location-create-cancel"
        >
          <X className="w-3 h-3" />
        </button>
      </div>

      <label className="block px-1 pb-0.5 text-[9px] font-mono text-slate-500" htmlFor="worktree-branch">
        Branch
      </label>
      <input
        id="worktree-branch"
        value={branch}
        onChange={(event) => setBranch(event.target.value)}
        disabled={busy}
        placeholder="feature/my-change"
        aria-label="Branch name"
        autoComplete="off"
        spellCheck={false}
        className="w-full rounded-md border border-white/10 bg-black/25 px-2 py-1.5 text-[11.5px] font-mono text-slate-200 outline-none focus:border-violet-400 disabled:opacity-40"
      />

      <label className="block px-1 pt-2 pb-0.5 text-[9px] font-mono text-slate-500" htmlFor="worktree-base">
        Based on
      </label>
      <select
        id="worktree-base"
        value={selectedBase}
        onChange={(event) => setBase(event.target.value)}
        disabled={busy || loadingBranches}
        aria-label="Base branch"
        className="w-full rounded-md border border-white/10 bg-[#0c0d12] px-2 py-1.5 text-[11.5px] font-mono text-slate-200 outline-none focus:border-violet-400 disabled:opacity-40"
        data-testid="terminal-location-base"
      >
        {branches.length === 0 && <option value={selectedBase}>{selectedBase}</option>}
        {branches.map((option) => (
          <option key={option.name} value={option.name}>
            {option.name}
            {option.hasRemote ? '' : '  · local only'}
          </option>
        ))}
      </select>

      <p
        className="flex items-start gap-1.5 px-1 pt-1.5 text-[10px] font-mono leading-relaxed text-slate-500"
        data-testid="terminal-location-base-note"
      >
        <RefreshCw className={`w-3 h-3 mt-px shrink-0 ${baseOption?.hasRemote === false ? 'text-amber-400/70' : 'text-cyan-400/70'}`} />
        {loadingBranches ? (
          <span>Reading branches…</span>
        ) : baseOption?.hasRemote === false ? (
          <span>
            No <span className="text-slate-400">origin</span> copy — starts from your local{' '}
            <span className="text-slate-400">{selectedBase}</span>.
          </span>
        ) : (
          <span>
            Fetches <span className="text-cyan-300">origin/{selectedBase}</span> first, so the
            branch starts up to date.
          </span>
        )}
      </p>

      <div className="flex items-center gap-1.5 px-1 pt-2 pb-0.5">
        <span className="text-[9px] font-mono text-slate-500">Folder</span>
        {folderOverride === null && (
          <button
            type="button"
            onClick={() => setFolderOverride(derived)}
            disabled={busy}
            className="inline-flex items-center gap-1 text-[9px] font-mono text-slate-600 hover:text-slate-300 transition disabled:opacity-40"
            data-testid="terminal-location-folder-edit"
          >
            <Pencil className="w-2.5 h-2.5" /> edit
          </button>
        )}
      </div>
      {folderOverride === null ? (
        <div className="px-1 text-[11px] font-mono text-slate-400 truncate" data-testid="terminal-location-folder">
          {derived || <span className="text-slate-600">named after the branch</span>}
        </div>
      ) : (
        <input
          value={folderOverride}
          onChange={(event) => setFolderOverride(event.target.value)}
          disabled={busy}
          aria-label="Worktree name"
          autoComplete="off"
          spellCheck={false}
          className="w-full rounded-md border border-white/10 bg-black/25 px-2 py-1.5 text-[11.5px] font-mono text-slate-200 outline-none focus:border-violet-400 disabled:opacity-40"
        />
      )}

      <button
        type="button"
        onClick={() => onSubmit({ branch: branch.trim(), baseBranch: selectedBase, worktreeName: folder })}
        disabled={!canSubmit}
        className="mt-2.5 w-full flex items-center justify-center gap-1.5 rounded-md bg-violet-600 px-2 py-1.5 text-[11.5px] font-mono text-white shadow-[0_0_15px_rgba(139,92,246,0.35)] hover:bg-violet-500 transition disabled:opacity-40 disabled:shadow-none"
        data-testid="terminal-location-create"
      >
        <Check className="w-3 h-3" />
        {busy ? 'Creating…' : 'Create worktree'}
      </button>

      {error && (
        <div
          className="mt-1.5 rounded border border-ruby-500/30 bg-ruby-500/10 px-2 py-1.5 text-[11px] font-mono text-ruby-300"
          data-testid="terminal-location-error"
        >
          {error}
        </div>
      )}
    </div>
  );
}
