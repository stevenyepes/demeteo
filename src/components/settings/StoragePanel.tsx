import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { AlertTriangle, HardDrive, RefreshCw, RotateCw, Trash2 } from 'lucide-react';

import { StorageProjectCard } from './StorageProjectCard';
import { Chip } from '../ui/Chip';
import { useProject } from '../../context';
import { viewSweep } from '../../lib/cacheSweepView';
import {
  describeSweepStatus,
  readCacheSweep,
  startCacheSweep,
  subscribeCacheSweep,
} from '../../lib/cacheSweepSession';

export function StoragePanel() {
  const { state } = useProject();
  const sweep = useSyncExternalStore(subscribeCacheSweep, readCacheSweep);
  const { busy, report, error, finishedAt } = sweep;
  const [confirming, setConfirming] = useState(false);

  const run = (dryRun: boolean) => {
    setConfirming(false);
    void startCacheSweep(dryRun);
  };

  useEffect(() => {
    const { busy, report } = readCacheSweep();
    if (!busy && !report) void startCacheSweep(true);
  }, []);

  const projectNames = useMemo(
    () => Object.fromEntries(state.projects.map(p => [p.id, p.name])),
    [state.projects],
  );
  const status = describeSweepStatus(sweep, projectNames);
  const view = useMemo(() => (report ? viewSweep(report, projectNames) : null), [report, projectNames]);
  const reclaimable = report?.dry_run ? view?.totals.delete ?? 0 : 0;

  return (
    <div className="space-y-4">
      <div className="glass-panel p-6 space-y-4">
        <div className="flex flex-col md:flex-row md:items-start justify-between gap-4">
          <div>
            <h3 className="text-sm font-heading font-semibold text-white mb-1 flex items-center gap-2">
              <HardDrive className="w-4 h-4 text-cyan-400" />
              Dependency Caches & Worktrees
            </h3>
            <p className="text-xs text-slate-400 max-w-2xl leading-relaxed">
              What runs, Ask threads and Discoveries left beside each project's clone, and whether any feature or session still needs it.
              A sweep also runs every 6 hours in the background; how long an idle feature's cache is kept is set per project under Agent Strategy & Policies.
            </p>
          </div>
          <div className="flex gap-2 shrink-0">
            <button
              type="button"
              onClick={() => run(true)}
              disabled={busy !== null}
              className="px-3 py-2 text-xs rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-white disabled:opacity-40 flex items-center gap-1.5 transition-all"
            >
              <RefreshCw className="w-3.5 h-3.5" /> Rescan
            </button>
            <button
              type="button"
              onClick={() => setConfirming(true)}
              disabled={busy !== null || reclaimable === 0}
              className="px-3 py-2 text-xs font-semibold rounded-lg bg-ruby-600 hover:bg-ruby-500 text-white disabled:opacity-40 disabled:hover:bg-ruby-600 flex items-center gap-1.5 transition-all"
            >
              <Trash2 className="w-3.5 h-3.5" /> Reclaim
            </button>
          </div>
        </div>

        {confirming && (
          <div role="alertdialog" aria-label="Confirm reclaim" className="flex flex-col md:flex-row md:items-center gap-3 p-3 rounded-lg bg-ruby-500/5 border border-ruby-500/20">
            <p className="text-xs text-ruby-200 flex-1 leading-relaxed">
              Delete the {reclaimable} {reclaimable === 1 ? 'entry' : 'entries'} this scan marked reclaimable? Each is judged again just before it goes, and one that has become needed since is spared. Undecided and kept entries are not touched.
            </p>
            <div className="flex gap-2 shrink-0">
              <button type="button" onClick={() => setConfirming(false)} className="px-3 py-1.5 text-xs rounded-md border border-white/10 text-slate-300 hover:bg-white/5 transition-all">
                Cancel
              </button>
              <button type="button" onClick={() => run(false)} className="px-3 py-1.5 text-xs font-semibold rounded-md bg-ruby-600 hover:bg-ruby-500 text-white transition-all">
                Delete {reclaimable}
              </button>
            </div>
          </div>
        )}

        {status && (
          <div role="status" className="flex items-start gap-2 text-sm text-slate-300">
            <RotateCw className="w-4 h-4 mt-0.5 animate-spin text-cyan-400 shrink-0" />
            <div className="min-w-0">
              <p>{status.title}</p>
              {status.detail && <p className="text-[11px] font-mono text-slate-400 mt-0.5 break-all">{status.detail}</p>}
              <p className="text-[11px] text-slate-500 mt-0.5">This can take minutes on a remote machine. It carries on if you leave this tab.</p>
            </div>
          </div>
        )}

        {error && (
          <div className="flex items-start gap-2 p-3 rounded-lg bg-ruby-500/10 border border-ruby-500/30">
            <AlertTriangle className="w-4 h-4 text-ruby-400 shrink-0" />
            <p className="text-xs text-ruby-200 break-words">{error}</p>
          </div>
        )}

        {view && !busy && (
          <div className="flex flex-wrap gap-2">
            {report?.dry_run ? (
              <>
                <Chip tone="violet">{view.totals.delete} reclaimable</Chip>
                <Chip tone="amber">{view.totals.unknown} undecided</Chip>
                <Chip tone="slate">{view.totals.keep} kept</Chip>
              </>
            ) : (
              <>
                <Chip tone="emerald">{view.totals.deleted} deleted</Chip>
                <Chip tone="ruby">{view.totals.failed} failed</Chip>
                <Chip tone="slate">{view.totals.spared} spared</Chip>
              </>
            )}
            {finishedAt !== null && (
              <span className="text-[11px] text-slate-500 self-center">
                as of {new Date(finishedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
              </span>
            )}
            {view.totals.projectErrors > 0 && (
              <Chip tone="ruby">{view.totals.projectErrors} project{view.totals.projectErrors === 1 ? '' : 's'} with errors</Chip>
            )}
          </div>
        )}
      </div>

      {view && (
        view.projects.length === 0 ? (
          <div className="glass-panel p-6 text-sm text-slate-500 italic">No project has a clone to sweep.</div>
        ) : (
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
            {view.projects.map(project => <StorageProjectCard key={project.projectId} project={project} />)}
          </div>
        )
      )}
    </div>
  );
}
