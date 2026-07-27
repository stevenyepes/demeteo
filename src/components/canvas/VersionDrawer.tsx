/**
 * `VersionDrawer` — the builder's version history (task P3.4, PRD §6.3).
 *
 * `workflow_versions` has existed since long before the builder did; every save
 * has been minting immutable rows that nothing ever showed. This is the missing
 * half of audit finding F39: list them, diff them on the canvas, and put an
 * author back on one.
 *
 * Three things it deliberately does not do:
 *
 *  - **It never edits.** Comparing hands the *builder* a read-only merged graph
 *    (`graphDiff.mergeForDiff`); restoring goes through `workflow_restore_version`,
 *    which copies the stored `steps_json` verbatim into a new row. Neither path
 *    routes an old version through the editor's v2 model, which today would
 *    down-project it lossily on the way back to storage.
 *  - **It never overwrites unsaved work.** Restore and revert replace the graph
 *    on the canvas, so both are refused while the editor is dirty — with the
 *    reason spelled out on the disabled button (PRD §6.4), the same shape the
 *    run-mode Actions tab uses for its ancestor guard.
 *  - **It never destroys a version.** Restore appends; "revert to default"
 *    appends too. The row you came from is still there afterwards, which is the
 *    property that makes trying a restore safe.
 */
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { GitCompare, History, RotateCcw, Undo2, X } from 'lucide-react';

import { formatError } from '../../lib/errors';
import { relativeTime } from '../../lib/utils';
import type { WorkflowDefinitionV2 } from './types';

/** Serde shape of the Rust `WorkflowVersion` row. */
export interface WorkflowVersionRow {
  id: string;
  workflow_id: string;
  version: number;
  steps_json: string;
  note: string | null;
  created_at: number;
}

/** One side of a comparison. A `null` graph means "the working copy", which
 *  only the builder holds. */
export interface ComparisonSide {
  versionId: string | null;
  label: string;
  graph: WorkflowDefinitionV2 | null;
}

export interface VersionComparison {
  /** The older side — verdicts read as "what `to` did to `from`". */
  from: ComparisonSide & { graph: WorkflowDefinitionV2 };
  to: ComparisonSide;
}

/** What landed after a restore or a revert, for the builder to adopt. */
export interface RestoredWorkflow {
  kind: 'restore' | 'revert';
  version: number;
  versionId: string;
  name: string;
  description: string;
  definition: WorkflowDefinitionV2;
  /** The version whose content was restored (absent for a revert). */
  sourceVersion?: number;
}

/** Serde shape of the Rust `WorkflowWithSteps` the write commands return. */
interface WorkflowResult {
  name: string;
  description: string;
  version: number;
  version_id: string;
}

export interface VersionDrawerProps {
  workflowId: string;
  /** Starters can be reverted to their bundled definition. */
  isStarter?: boolean;
  /** The editor has unsaved edits — restore/revert would discard them. */
  dirty: boolean;
  /** Changes whenever the owner writes a version, to re-read the list. */
  reloadToken?: number;
  comparison: VersionComparison | null;
  onCompare: (comparison: VersionComparison | null) => void;
  onRestored: (result: RestoredWorkflow) => void;
  onClose: () => void;
  className?: string;
}

/** Which side a comparison is measured against; `working` = the live editor. */
type CompareTarget = 'working' | string;

const DIRTY_REASON =
  'Save or discard your unsaved edits first — restoring replaces the graph on the canvas.';

export function VersionDrawer({
  workflowId,
  isStarter = false,
  dirty,
  reloadToken = 0,
  comparison,
  onCompare,
  onRestored,
  onClose,
  className = '',
}: VersionDrawerProps) {
  const [versions, setVersions] = useState<WorkflowVersionRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /** Version id of an in-flight restore, or `'revert'`. */
  const [busy, setBusy] = useState<string | null>(null);
  const [target, setTarget] = useState<CompareTarget>('working');

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const rows = await invoke<WorkflowVersionRow[]>('workflow_versions', { workflowId });
      // Newest first: history is read backwards from where you are.
      setVersions([...rows].sort((a, b) => b.version - a.version));
      setError(null);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setLoading(false);
    }
  }, [workflowId]);

  useEffect(() => {
    void load();
  }, [load, reloadToken]);

  const graphFor = useCallback(
    (versionId: string) =>
      invoke<WorkflowDefinitionV2>('workflow_version_graph', { workflowId, versionId }),
    [workflowId],
  );

  const latest = versions[0];

  // ── Compare ─────────────────────────────────────────────────────────────

  const runCompare = useCallback(
    async (row: WorkflowVersionRow, against: CompareTarget) => {
      try {
        const from = await graphFor(row.id);
        const to: ComparisonSide =
          against === 'working'
            ? { versionId: null, label: 'Working copy', graph: null }
            : {
                versionId: against,
                label: `v${versions.find((v) => v.id === against)?.version ?? '?'}`,
                graph: await graphFor(against),
              };
        onCompare({ from: { versionId: row.id, label: `v${row.version}`, graph: from }, to });
        setError(null);
      } catch (err) {
        setError(formatError(err));
      }
    },
    [graphFor, versions, onCompare],
  );

  const toggleCompare = useCallback(
    (row: WorkflowVersionRow) => {
      if (comparison?.from.versionId === row.id) onCompare(null);
      else void runCompare(row, target);
    },
    [comparison, onCompare, runCompare, target],
  );

  /** Re-run the active comparison against a newly chosen other side. */
  const retarget = useCallback(
    (next: CompareTarget) => {
      setTarget(next);
      const active = versions.find((v) => v.id === comparison?.from.versionId);
      if (active) void runCompare(active, next);
    },
    [versions, comparison, runCompare],
  );

  // ── Restore / revert ────────────────────────────────────────────────────

  /** Both writes end the same way: adopt the new version, drop the compare
   *  view (it is measured against a graph that no longer exists), reload. */
  const adopt = useCallback(
    async (result: WorkflowResult, kind: 'restore' | 'revert', sourceVersion?: number) => {
      const definition = await graphFor(result.version_id);
      onCompare(null);
      onRestored({
        kind,
        version: result.version,
        versionId: result.version_id,
        name: result.name,
        description: result.description,
        definition,
        sourceVersion,
      });
      await load();
    },
    [graphFor, onCompare, onRestored, load],
  );

  const restore = useCallback(
    async (row: WorkflowVersionRow) => {
      setBusy(row.id);
      try {
        const result = await invoke<WorkflowResult>('workflow_restore_version', {
          workflowId,
          versionId: row.id,
        });
        await adopt(result, 'restore', row.version);
        setError(null);
      } catch (err) {
        setError(formatError(err));
      } finally {
        setBusy(null);
      }
    },
    [workflowId, adopt],
  );

  const revert = useCallback(async () => {
    setBusy('revert');
    try {
      const result = await invoke<WorkflowResult>('workflow_revert_to_default', { workflowId });
      await adopt(result, 'revert');
      setError(null);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setBusy(null);
    }
  }, [workflowId, adopt]);

  // ── Render ──────────────────────────────────────────────────────────────

  return (
    <aside
      className={`flex h-full w-[340px] shrink-0 flex-col border-l border-white/5 bg-[#0d0f14]/80 backdrop-blur-xl ${className}`}
      data-testid="version-drawer"
      aria-label="Version history"
    >
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-white/5 px-4 py-3">
        <div className="flex items-center gap-2">
          <History className="h-4 w-4 text-slate-400" aria-hidden />
          <h3 className="font-display text-sm font-bold uppercase tracking-wider text-white">
            Version history
          </h3>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close version history"
          className="rounded-lg p-1.5 text-slate-500 transition-colors hover:bg-white/5 hover:text-slate-200"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-4 py-2 text-[11px] text-slate-400">
        <GitCompare className="h-3.5 w-3.5 shrink-0" aria-hidden />
        <label htmlFor="compare-target" className="shrink-0">
          Compare against
        </label>
        <select
          id="compare-target"
          value={target}
          onChange={(e) => retarget(e.target.value)}
          className="min-w-0 flex-1 rounded border border-slate-700/60 bg-slate-900/60 px-1.5 py-1 text-[11px] text-slate-200 outline-none focus:border-cyan-500/50"
        >
          <option value="working">Working copy</option>
          {versions.map((v) => (
            <option key={v.id} value={v.id}>
              v{v.version}
            </option>
          ))}
        </select>
      </div>

      {error && (
        <p className="border-b border-rose-500/20 bg-rose-500/5 px-4 py-2 text-xs text-rose-200">
          {error}
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        {loading ? (
          <p className="px-1 py-2 text-xs text-slate-500">Loading versions…</p>
        ) : versions.length === 0 ? (
          <p className="px-1 py-2 text-xs text-slate-500">
            No saved versions yet. The first save creates v1.
          </p>
        ) : (
          <ul className="space-y-2">
            {versions.map((row) => {
              const comparing = comparison?.from.versionId === row.id;
              const isLatest = row.id === latest?.id;
              const restoreBlocked = dirty
                ? DIRTY_REASON
                : isLatest
                  ? 'This is the current version.'
                  : null;
              return (
                <li
                  key={row.id}
                  data-testid={`version-row-${row.version}`}
                  className={[
                    'rounded-lg border px-3 py-2 transition-colors',
                    comparing
                      ? 'border-amber-500/40 bg-amber-500/5'
                      : 'border-slate-700/50 bg-slate-900/40',
                  ].join(' ')}
                >
                  <div className="flex items-baseline gap-2">
                    <span className="font-mono text-xs font-semibold text-slate-100">
                      v{row.version}
                    </span>
                    {isLatest && (
                      <span className="rounded border border-cyan-500/30 bg-cyan-500/10 px-1 py-px text-[9px] font-bold uppercase tracking-wide text-cyan-300">
                        Current
                      </span>
                    )}
                    <span className="ml-auto shrink-0 text-[10px] text-slate-500">
                      {relativeTime(row.created_at)}
                    </span>
                  </div>
                  {row.note && (
                    <p className="mt-0.5 truncate text-[11px] text-slate-400" title={row.note}>
                      {row.note}
                    </p>
                  )}
                  <div className="mt-2 flex items-center gap-1.5">
                    <button
                      type="button"
                      onClick={() => toggleCompare(row)}
                      className={[
                        'flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] font-medium transition-colors',
                        comparing
                          ? 'border-amber-500/50 bg-amber-500/10 text-amber-200'
                          : 'border-slate-700/60 text-slate-300 hover:border-slate-600 hover:text-white',
                      ].join(' ')}
                    >
                      <GitCompare className="h-3 w-3" />
                      {comparing ? 'Stop comparing' : 'Compare'}
                    </button>
                    <button
                      type="button"
                      onClick={() => void restore(row)}
                      disabled={Boolean(restoreBlocked) || busy !== null}
                      title={restoreBlocked ?? `Restore v${row.version} as a new version`}
                      className="flex items-center gap-1 rounded border border-slate-700/60 px-1.5 py-0.5 text-[10px] font-medium text-slate-300 transition-colors hover:border-slate-600 hover:text-white disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      <Undo2 className="h-3 w-3" />
                      {busy === row.id ? 'Restoring…' : 'Restore'}
                    </button>
                  </div>
                  {restoreBlocked === DIRTY_REASON && (
                    <p className="mt-1 text-[10px] leading-snug text-amber-300/80">
                      {DIRTY_REASON}
                    </p>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {isStarter && (
        <div className="shrink-0 border-t border-white/5 px-4 py-3">
          <button
            type="button"
            onClick={() => void revert()}
            disabled={dirty || busy !== null}
            title={dirty ? DIRTY_REASON : 'Save the bundled starter definition as a new version'}
            className="flex w-full items-center justify-center gap-1.5 rounded-lg border border-slate-700/60 px-2 py-1.5 text-[11px] font-medium text-slate-300 transition-colors hover:border-slate-600 hover:text-white disabled:cursor-not-allowed disabled:opacity-40"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            {busy === 'revert' ? 'Reverting…' : 'Revert to default'}
          </button>
          <p className="mt-1.5 text-[10px] leading-snug text-slate-500">
            Appends the bundled definition as a new version. Your existing versions stay.
          </p>
        </div>
      )}
    </aside>
  );
}
