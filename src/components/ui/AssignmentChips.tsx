import React from 'react';

import type { EffortLevel } from '../../lib/effortLevels';
import { assignmentAriaLabel, assignmentEffortLabel } from '../../lib/runEventAssignments';

interface AssignmentChipsProps {
  /** What the badges annotate, for the accessible name: a node title, a step name. */
  subject: string;
  /** The spawned agent, or null/absent when the run left no spawn evidence. */
  agentKind?: string | null;
  /** `null` = the spawn injected no effort; absent = no evidence either way. */
  effort?: EffortLevel | null;
  className?: string;
}

const CHIP =
  'flex min-w-0 items-center gap-1 rounded border border-slate-600/40 bg-slate-700/20 px-1.5 py-0.5 text-slate-300';

/**
 * What a run *actually* spawned for one step execution — the agent and the
 * post-clamp effort — as the canvas and the timeline both draw it.
 *
 * One component rather than one per surface: it is the same fact about the
 * same execution, and the two had already drifted into different palettes,
 * which reads as two different data. Slate because this is an annotation and
 * not a state — §4 spends cyan and emerald on what a run is *doing*, and a
 * chip that borrows those colours competes with the status language beside it.
 *
 * The pair is announced once, as one composed label on one `role="img"`: the
 * badges are two halves of a single reading, and nesting a labelled group per
 * badge made a screen reader say the assignment three times.
 */
export function AssignmentChips({
  subject,
  agentKind,
  effort,
  className = '',
}: AssignmentChipsProps): React.ReactElement | null {
  const observedAgent =
    typeof agentKind === 'string' && agentKind.trim().length > 0 ? agentKind : null;
  if (!observedAgent || effort === undefined) return null;

  const effortLabel = assignmentEffortLabel(effort);
  return (
    <span
      role="img"
      aria-label={assignmentAriaLabel(subject, observedAgent, effortLabel)}
      className={`flex min-w-0 flex-wrap items-center gap-1 font-mono text-[9px] ${className}`}
    >
      <span className={`max-w-[160px] ${CHIP}`} title={`Agent: ${observedAgent}`}>
        <span className="shrink-0 text-slate-400" aria-hidden>
          Agent
        </span>
        <span className="truncate">{observedAgent}</span>
      </span>
      <span className={CHIP} title={`Effective effort: ${effortLabel}`}>
        <span className="shrink-0 text-slate-400" aria-hidden>
          Effort
        </span>
        <span className="truncate">{effortLabel}</span>
      </span>
    </span>
  );
}
