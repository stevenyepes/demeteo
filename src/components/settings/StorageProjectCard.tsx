import { AlertTriangle, CircleCheck, CircleX, ShieldCheck } from 'lucide-react';

import { Chip } from '../ui/Chip';
import type { SweepProjectView } from '../../lib/cacheSweepView';
import type { CacheSweepEntry, CacheSweepVerdict } from '../../lib/featureDetail';
import type { RunStatusTone } from '../../lib/runStatus';

const VERDICT_LABEL: Record<CacheSweepVerdict, { label: string; tone: RunStatusTone }> = {
  delete: { label: 'Reclaimable', tone: 'violet' },
  unknown: { label: 'Undecided — left for you', tone: 'amber' },
  keep: { label: 'Kept', tone: 'slate' },
};

function Outcome({ outcome }: { outcome: CacheSweepEntry['outcome'] }) {
  if (!outcome) return null;
  if (outcome.status === 'deleted') {
    return (
      <span className="flex items-center gap-1 text-[11px] text-emerald-400">
        <CircleCheck className="w-3 h-3" /> Deleted
      </span>
    );
  }
  if (outcome.status === 'failed') {
    return (
      <span className="flex items-start gap-1 text-[11px] text-ruby-400 break-words">
        <CircleX className="w-3 h-3 mt-0.5 shrink-0" /> Failed: {outcome.detail}
      </span>
    );
  }
  return (
    <span className="flex items-start gap-1 text-[11px] text-slate-300">
      <ShieldCheck className="w-3 h-3 mt-0.5 shrink-0" /> Spared: {outcome.detail.text}
    </span>
  );
}

function EntryRow({ entry }: { entry: CacheSweepEntry }) {
  return (
    <li className="flex flex-col gap-1 py-2 border-t border-white/5 first:border-t-0">
      <div className="flex items-center gap-2 min-w-0">
        <Chip tone={entry.kind === 'cache' ? 'cyan' : 'violet'} size="sm">{entry.kind}</Chip>
        <span className="font-mono text-xs text-slate-200 truncate min-w-0" title={entry.path}>{entry.path}</span>
      </div>
      <p className="text-[11px] text-slate-500 leading-relaxed">{entry.reason.text}</p>
      <Outcome outcome={entry.outcome} />
    </li>
  );
}

export function StorageProjectCard({ project }: { project: SweepProjectView }) {
  return (
    <div className="nested-card p-4 space-y-3">
      <div className="min-w-0">
        <h4 className="font-heading text-sm font-semibold text-white truncate">{project.name}</h4>
        {project.cloneDir && (
          <p className="font-mono text-[11px] text-slate-500 truncate" title={project.cloneDir}>{project.cloneDir}</p>
        )}
      </div>

      {project.error && (
        <div className="flex items-start gap-2 p-2.5 rounded-lg bg-ruby-500/5 border border-ruby-500/20">
          <AlertTriangle className="w-4 h-4 text-ruby-400 shrink-0" />
          <p className="text-xs text-ruby-300 break-words">Not scanned: {project.error}</p>
        </div>
      )}
      {project.worktreeListError && (
        <div className="flex items-start gap-2 p-2.5 rounded-lg bg-ruby-500/5 border border-ruby-500/20">
          <AlertTriangle className="w-4 h-4 text-ruby-400 shrink-0" />
          <p className="text-xs text-ruby-300 break-words">Every worktree kept — git could not list them: {project.worktreeListError}</p>
        </div>
      )}

      {project.groups.length === 0 && !project.error && (
        <p className="text-xs text-slate-500 italic">Nothing beside this clone.</p>
      )}
      {project.groups.map(group => (
        <section key={group.verdict} className="space-y-1">
          <Chip tone={VERDICT_LABEL[group.verdict].tone} size="sm">
            {VERDICT_LABEL[group.verdict].label} · {group.entries.length}
          </Chip>
          <ul>
            {group.entries.map(entry => <EntryRow key={entry.path} entry={entry} />)}
          </ul>
        </section>
      ))}
    </div>
  );
}
