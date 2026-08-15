import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react';
import { GitBranch } from 'lucide-react';

import type { TerminalBranchOption } from '../../types';
import { listTerminalBranches } from '../../lib/terminal';
import { formatError } from '../../lib/errors';
import { Disclosure } from '../ui/Disclosure';
import type { OriginSelection } from '../../lib/runOrigin';

export interface OriginPickerProps {
  projectId: string;
  /** The repository whose refs are listed. `null` while no repo is selected —
   *  branch names are per-repository, so there is nothing truthful to offer
   *  until one is. */
  repositoryId: string | null;
  value: OriginSelection;
  onChange: (selection: OriginSelection) => void;
  disabled?: boolean;
}

const UNSTATED = '';

/**
 * Where a run starts and what it is measured against, as a collapsed section.
 *
 * Collapsed and stating a default is the whole design: every run before this
 * control started at the project's default branch and none of them asked, so
 * an expanded pair of selects would turn a launch that needs no decision into
 * one that presents two. The summary line is what keeps that honest — a user
 * who never opens this still reads what it chose for them.
 *
 * Refs are read on first open rather than on mount, for the same reason
 * `TerminalWorktreeLocationPicker` defers its own: listing refs is a round
 * trip to a possibly remote host, and the closed state needs none of it.
 */
export function OriginPicker({
  projectId,
  repositoryId,
  value,
  onChange,
  disabled = false,
}: OriginPickerProps): ReactElement {
  const [open, setOpen] = useState(false);
  const [branches, setBranches] = useState<TerminalBranchOption[]>([]);
  const [defaultBranch, setDefaultBranch] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const targetKey = `${projectId}:${repositoryId ?? ''}`;
  const currentTarget = useRef(targetKey);
  currentTarget.current = targetKey;

  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  // A branch named for one repository is not a branch in the next, so the
  // selection cannot survive a retarget — it would launch a run from a base
  // that resolves nowhere.
  useEffect(() => {
    setBranches([]);
    setDefaultBranch(null);
    setError(null);
    onChangeRef.current({ base: null, diffBase: null });
  }, [targetKey]);

  const load = useCallback(async () => {
    if (!projectId || !repositoryId) return;
    const requested = `${projectId}:${repositoryId}`;
    setLoading(true);
    setError(null);
    try {
      const options = await listTerminalBranches(projectId, repositoryId);
      if (requested !== currentTarget.current) return;
      setBranches(options.branches);
      setDefaultBranch(options.defaultBranch);
    } catch (err) {
      if (requested === currentTarget.current) setError(formatError(err));
    } finally {
      if (requested === currentTarget.current) setLoading(false);
    }
  }, [projectId, repositoryId]);

  const toggle = useCallback(
    (next: boolean) => {
      setOpen(next);
      if (next && branches.length === 0) void load();
    },
    [branches.length, load],
  );

  const defaultLabel = defaultBranch ?? "the project's default branch";
  const startsFrom = value.base ?? defaultLabel;
  const summary =
    value.base === null && value.diffBase === null
      ? "Starts from the project's default branch"
      : `Starts from ${startsFrom}, diffed against ${value.diffBase ?? startsFrom}`;

  const selectClass =
    'w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 disabled:opacity-40';

  const branchOptions = branches.map((option) => (
    <option key={option.name} value={option.name}>
      {option.name}
      {option.hasRemote ? '' : '  · local only'}
    </option>
  ));

  return (
    <Disclosure
      title="Start point"
      open={open}
      onOpenChange={toggle}
      icon={<GitBranch className="w-3.5 h-3.5 text-violet-400" />}
      meta={
        open ? undefined : (
          <span
            className="text-[11px] font-mono text-slate-500 truncate"
            data-testid="origin-picker-summary"
          >
            {summary}
          </span>
        )
      }
      bodyClassName="p-4 space-y-3"
    >
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label
            htmlFor="start-feature-origin-base"
            className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider"
          >
            Start from
          </label>
          <select
            id="start-feature-origin-base"
            value={value.base ?? UNSTATED}
            disabled={disabled || loading || !repositoryId}
            onChange={(event) =>
              onChange({ ...value, base: event.target.value || null })
            }
            className={selectClass}
          >
            <option value={UNSTATED}>{defaultBranch ?? 'Default branch'}</option>
            {branchOptions}
          </select>
        </div>
        <div>
          <label
            htmlFor="start-feature-origin-diff-base"
            className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider"
          >
            Diff against
          </label>
          <select
            id="start-feature-origin-diff-base"
            value={value.diffBase ?? UNSTATED}
            disabled={disabled || loading || !repositoryId}
            onChange={(event) =>
              onChange({ ...value, diffBase: event.target.value || null })
            }
            className={selectClass}
          >
            <option value={UNSTATED}>Same as start point</option>
            {branchOptions}
          </select>
        </div>
      </div>

      <p className="text-[10px] font-mono leading-relaxed text-slate-500">
        The run cuts its branch from the first and measures its changes against the second.
        Leave both alone for a normal run.
      </p>

      {!repositoryId && (
        <p className="text-[10px] font-mono text-amber-300">
          Select a repository to list its branches.
        </p>
      )}
      {loading && <p className="text-[10px] font-mono text-slate-500">Reading branches…</p>}
      {error && (
        <p role="alert" className="text-[11px] font-mono text-ruby-200">
          {error}
        </p>
      )}
    </Disclosure>
  );
}
