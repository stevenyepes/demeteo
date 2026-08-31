import React from 'react';
import { FilePlus2, FileText, Globe, Loader, Pencil, Terminal } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import { describeTool, formatActivitySummary, type ActivityKind, type LiveTurn } from '../../lib/askActivity';
import { formatDuration } from '../../lib/utils';

interface AskActivityStripProps {
  turn: LiveTurn;
  elapsedMs: number;
}

const ICONS: Record<ActivityKind, LucideIcon> = {
  read: FileText,
  edit: Pencil,
  write: FilePlus2,
  run_bash: Terminal,
  fetch: Globe,
};

/**
 * `discovery/TurnActivityStrip.tsx`'s shape, extended with the one
 * `ActivityKind` Ask has that Discovery does not — see
 * `src/lib/askActivity.ts`'s module doc for why `fetch` is derived rather
 * than carried on the wire.
 */
export function AskActivityStrip({ turn, elapsedMs }: AskActivityStripProps): React.ReactElement {
  const Icon = turn.current ? ICONS[turn.current.kind] : Loader;
  const doing = turn.current
    ? describeTool(turn.current)
    : turn.text.length > 0
      ? 'Answering'
      : turn.phase === 'setting_up'
        ? 'Preparing the turn'
        : 'Thinking';
  const summary = formatActivitySummary(turn.activity);

  return (
    <div
      data-testid="turn-activity"
      className="mb-2 rounded-md border border-white/5 bg-white/[0.02] px-2.5 py-1.5"
    >
      <div className="flex items-center gap-2">
        <Icon
          aria-hidden="true"
          className={`h-3.5 w-3.5 shrink-0 ${turn.current ? 'text-cyan-300' : 'animate-spin text-emerald-400 motion-reduce:animate-none'}`}
        />
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-slate-300">{doing}</span>
        {turn.alsoRunning > 0 && (
          <span className="shrink-0 font-mono text-[10px] text-slate-500">+{turn.alsoRunning}</span>
        )}
        <span
          data-testid="turn-elapsed"
          className="shrink-0 font-mono text-[10px] tabular-nums text-emerald-400/80"
        >
          {formatDuration(elapsedMs / 1000)}
        </span>
      </div>
      {summary && (
        <p data-testid="turn-activity-summary" className="mt-1 mb-0 font-mono text-[10px] text-slate-500">
          {summary}
        </p>
      )}
    </div>
  );
}

export default AskActivityStrip;
