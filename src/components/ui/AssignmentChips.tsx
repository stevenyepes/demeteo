import { Bot, Gauge, Zap } from 'lucide-react';
import React from 'react';

import type { EffortLevel } from '../../lib/effortLevels';
import {
  assignmentAriaLabel,
  assignmentEffortLabel,
  assignmentModelLabel,
} from '../../lib/runEventAssignments';

interface AssignmentChipsProps {
  /** What the badges annotate, for the accessible name: a node title, a step name. */
  subject: string;
  /** The spawned agent, or null/absent when the run left no spawn evidence. */
  agentKind?: string | null;
  /**
   * The pinned/resolved model; `null` = nothing pinned, the harness chose its
   * own; absent = no evidence either way, so no model chip is drawn.
   */
  model?: string | null;
  /** `null` = the spawn injected no effort; absent = no evidence either way. */
  effort?: EffortLevel | null;
  className?: string;
}

const CHIP =
  'flex min-w-0 items-center gap-1 rounded border border-slate-600/40 bg-slate-700/20 px-1.5 py-0.5 text-slate-300';

const ICON = 'h-2.5 w-2.5 shrink-0 text-slate-400';

/**
 * What a run *actually* spawned for one step execution — the agent, the model
 * and the post-clamp effort — as the canvas and the timeline both draw it.
 *
 * One component rather than one per surface: it is the same fact about the
 * same execution, and the two had already drifted into different palettes,
 * which reads as two different data. Slate because this is an annotation and
 * not a state — §4 spends cyan and emerald on what a run is *doing*, and a
 * chip that borrows those colours competes with the status language beside it.
 *
 * The trio is announced once, as one composed label on one `role="img"`: the
 * badges are parts of a single reading, and nesting a labelled group per
 * badge made a screen reader say the assignment several times over.
 *
 * Each chip is an icon plus its value, not a prefix word: `Bot` for the agent,
 * `Zap` for the model, `Gauge` for the effort. `Zap` and `Gauge` are what
 * `HarnessModelPicker.tsx` and the wizard's model step use where the user
 * *chooses* them, so the read-only record speaks the picker's language. The
 * picker's `Cpu` is deliberately not the agent glyph: `Cpu` is the spinning
 * running-status icon in the same `StepCard` row, and one glyph with two
 * meanings on one card is the worst collision available.
 *
 * The group never wraps internally, and the model chip gives way first. It
 * grows from a zero basis into whatever width agent and effort leave, up to
 * its own content, so a short row truncates the model while the other two stay
 * whole; they shrink only once the model is down to its padding. Not
 * `shrink-0` on those two instead: `claude-code` and `No injected effort`
 * alone outrun a clamped graph card, and every chip keeps `min-w-0` so the row
 * cannot spill past the card edge. Nor a heavier `shrink-[N]` on the model: it
 * still takes a fraction of a pixel from the others, which is enough to draw an
 * ellipsis. The model's 160 px cap sits on its value (132 px plus 28 px of
 * chrome) because its box is already capped at its content, and it clips
 * rather than letting its icon paint over its own border once the row leaves
 * it narrower than that icon. On the graph card this costs no height; in
 * `StepCard`'s wrapping row the group can drop onto a line of its own, as a
 * unit. The full value lives in each chip's `title` and in the composed label.
 */
export function AssignmentChips({
  subject,
  agentKind,
  model,
  effort,
  className = '',
}: AssignmentChipsProps): React.ReactElement | null {
  const observedAgent =
    typeof agentKind === 'string' && agentKind.trim().length > 0 ? agentKind : null;
  if (!observedAgent || effort === undefined) return null;

  const effortLabel = assignmentEffortLabel(effort);
  const modelLabel = model === undefined ? undefined : assignmentModelLabel(model);
  return (
    <span
      role="img"
      aria-label={assignmentAriaLabel(subject, observedAgent, effortLabel, modelLabel)}
      className={`flex min-w-0 flex-nowrap items-center gap-1 font-mono text-[9px] ${className}`}
    >
      <span className={`max-w-[160px] ${CHIP}`} title={`Agent: ${observedAgent}`}>
        <Bot className={ICON} aria-hidden="true" />
        <span className="truncate">{observedAgent}</span>
      </span>
      {modelLabel !== undefined && (
        <span
          className={`grow basis-0 max-w-max overflow-hidden ${CHIP}`}
          title={`Model: ${modelLabel}`}
        >
          <Zap className={ICON} aria-hidden="true" />
          <span className="max-w-[132px] truncate">{modelLabel}</span>
        </span>
      )}
      <span className={`max-w-[160px] ${CHIP}`} title={`Effective effort: ${effortLabel}`}>
        <Gauge className={ICON} aria-hidden="true" />
        <span className="truncate">{effortLabel}</span>
      </span>
    </span>
  );
}
