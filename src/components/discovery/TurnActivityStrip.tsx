import React from 'react';
import { FilePlus2, FileText, Loader, Pencil, Terminal } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import {
  describeTool,
  formatActivitySummary,
  type ActivityKind,
  type LiveTurn,
} from '../../lib/discoveryActivity';
import { formatDuration } from '../../lib/utils';

interface TurnActivityStripProps {
  turn: LiveTurn;
  elapsedMs: number;
}

const ICONS: Record<ActivityKind, LucideIcon> = {
  read: FileText,
  edit: Pencil,
  write: FilePlus2,
  run_bash: Terminal,
};

/**
 * What the turn is doing right now, and what it has done so far.
 *
 * It renders unconditionally, because the state it exists for is the one with
 * nothing in it: Claude's `thinking` blocks never reach the wire, so a turn can
 * spend minutes alive without emitting a single event. An elapsed clock that
 * only appears once something arrives would be absent for exactly the wait it
 * was added to explain.
 */
export function TurnActivityStrip({
  turn,
  elapsedMs,
}: TurnActivityStripProps): React.ReactElement {
  const Icon = turn.current ? ICONS[turn.current.kind] : Loader;
  const doing = turn.current
    ? describeTool(turn.current)
    : turn.text.length > 0
      ? 'Answering'
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
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-slate-300">
          {doing}
        </span>
        {turn.alsoRunning > 0 && (
          <span className="shrink-0 font-mono text-[10px] text-slate-500">
            +{turn.alsoRunning}
          </span>
        )}
        <span
          data-testid="turn-elapsed"
          className="shrink-0 font-mono text-[10px] tabular-nums text-emerald-400/80"
        >
          {formatDuration(elapsedMs / 1000)}
        </span>
      </div>
      {summary && (
        <p
          data-testid="turn-activity-summary"
          className="mt-1 mb-0 font-mono text-[10px] text-slate-500"
        >
          {summary}
        </p>
      )}
    </div>
  );
}

export default TurnActivityStrip;
