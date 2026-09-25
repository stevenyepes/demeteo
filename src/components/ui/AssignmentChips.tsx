import { Bot, Gauge, Zap, type LucideIcon } from 'lucide-react';
import React from 'react';

import { EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
import {
  assignmentAriaLabel,
  assignmentEffortLabel,
  assignmentModelLabel,
} from '../../lib/runEventAssignments';

interface AssignmentChipsProps {
  /** What the badges annotate, for the accessible name: a node title, a step name. */
  subject: string;
  /**
   * Which reading the three values below carry. `observed` is spawn evidence —
   * what a step execution actually ran with. `planned` is the step's pin before
   * anything has spawned, where each value is independently "inherit" when
   * `null` or absent, exactly as `StepOverride` states it.
   */
  variant?: 'observed' | 'planned';
  /** The spawned agent, or null/absent when the run left no spawn evidence. */
  agentKind?: string | null;
  /**
   * The resolved/pinned model; observed: `null` = nothing pinned, the harness
   * chose its own, absent = no evidence either way, so no model chip is drawn.
   */
  model?: string | null;
  /** Observed: `null` = the spawn injected no effort; absent = no evidence either way. */
  effort?: EffortLevel | null;
  className?: string;
}

const CHIP =
  'flex min-w-0 items-center gap-1 rounded border border-slate-600/40 bg-slate-700/20 px-1.5 py-0.5 text-slate-300';

const ICON = 'h-2.5 w-2.5 shrink-0 text-slate-400';

const PLANNED_CHIP =
  'flex min-w-0 items-center gap-1 rounded border border-dashed border-slate-600/40 bg-slate-700/10 px-1.5 py-0.5 text-slate-400';

const PLANNED_ICON = 'h-2.5 w-2.5 shrink-0 text-slate-500';

const BOX = 'max-w-[160px]';
const MODEL_BOX = 'grow basis-0 max-w-max overflow-hidden';
const TEXT = 'truncate';
const MODEL_TEXT = 'max-w-[132px] truncate';

interface Chip {
  Icon: LucideIcon;
  title: string;
  value: string;
  /** Layout, which is the dimension's own and not the variant's. */
  box: string;
  text: string;
}

/** A pinned dimension, before its two spellings — chip title and label segment. */
interface Dimension {
  Icon: LucideIcon;
  name: string;
  value: string;
  box: string;
  text: string;
}

function pinned(value: string | null | undefined): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null;
}

/**
 * What a run *actually* spawned for one step execution — the agent, the model
 * and the post-clamp effort — as the canvas and the timeline both draw it, and
 * what a step is *pinned to* before it has spawned anything.
 *
 * One component rather than one per surface: it is the same fact about the
 * same execution, and the two had already drifted into different palettes,
 * which reads as two different data. Slate because this is an annotation and
 * not a state — §4 spends cyan and emerald on what a run is *doing*, and a
 * chip that borrows those colours competes with the status language beside it.
 *
 * The two readings are not interchangeable and the rendering must never let
 * one be mistaken for the other: a planned chip asserting what ran is a chip
 * that lies about an execution. So they are separated twice over — dashed and
 * dimmed against solid for the eye, `Agent (planned)` / `Planned assignment
 * for …` against `Agent` / `Actual assignment for …` for a screen reader —
 * because either separation alone is invisible to half the audience. The
 * observed wording also outranks the planned one in precision and keeps it:
 * `Effective effort` is post-clamp and only a spawn can say it, so a pin is
 * announced as plain `Effort`.
 *
 * A planned reading draws only the dimensions the pin actually set, and
 * nothing at all when it set none — `null` is "inherit" there, not a value, so
 * `Harness default` and `No injected effort` (which are observations) never
 * appear on it.
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
export function AssignmentChips(props: AssignmentChipsProps): React.ReactElement | null {
  const reading = props.variant === 'planned' ? plannedReading(props) : observedReading(props);
  if (!reading) return null;

  const { chips, label, tone } = reading;
  return (
    <span
      role="img"
      aria-label={label}
      className={`flex min-w-0 flex-nowrap items-center gap-1 font-mono text-[9px] ${props.className ?? ''}`}
    >
      {chips.map((chip) => (
        <span key={chip.title} className={`${chip.box} ${tone.chip}`} title={chip.title}>
          <chip.Icon className={tone.icon} aria-hidden="true" />
          <span className={chip.text}>{chip.value}</span>
        </span>
      ))}
    </span>
  );
}

interface Reading {
  chips: Chip[];
  label: string;
  tone: { chip: string; icon: string };
}

/**
 * The spawn evidence in a step's agent/effort pair, or `null` where there is
 * none — the same question [`AssignmentChips`] answers before it draws an
 * observed trio, exported so a run surface choosing between the two variants
 * chooses by the rule that renders them. A surface that decides this for
 * itself can disagree with the component and draw neither reading.
 */
export function observedAssignment(
  agentKind: string | null | undefined,
  effort: EffortLevel | null | undefined,
): { agentKind: string; effort: EffortLevel | null } | null {
  const agent = pinned(agentKind);
  return agent !== null && effort !== undefined ? { agentKind: agent, effort } : null;
}

function observedReading({ subject, agentKind, model, effort }: AssignmentChipsProps): Reading | null {
  const observed = observedAssignment(agentKind, effort);
  if (!observed) return null;
  const observedAgent = observed.agentKind;

  const effortLabel = assignmentEffortLabel(observed.effort);
  const modelLabel = model === undefined ? undefined : assignmentModelLabel(model);
  const chips: Chip[] = [
    { Icon: Bot, title: `Agent: ${observedAgent}`, value: observedAgent, box: BOX, text: TEXT },
  ];
  if (modelLabel !== undefined) {
    chips.push({
      Icon: Zap,
      title: `Model: ${modelLabel}`,
      value: modelLabel,
      box: MODEL_BOX,
      text: MODEL_TEXT,
    });
  }
  chips.push({
    Icon: Gauge,
    title: `Effective effort: ${effortLabel}`,
    value: effortLabel,
    box: BOX,
    text: TEXT,
  });

  return {
    chips,
    label: assignmentAriaLabel(subject, observedAgent, effortLabel, modelLabel),
    tone: { chip: CHIP, icon: ICON },
  };
}

function plannedReading({ subject, agentKind, model, effort }: AssignmentChipsProps): Reading | null {
  const dimensions: Dimension[] = [];
  const agent = pinned(agentKind);
  if (agent) dimensions.push({ Icon: Bot, name: 'Agent', value: agent, box: BOX, text: TEXT });
  const pinnedModel = pinned(model);
  if (pinnedModel) {
    dimensions.push({
      Icon: Zap,
      name: 'Model',
      value: pinnedModel,
      box: MODEL_BOX,
      text: MODEL_TEXT,
    });
  }
  if (effort !== null && effort !== undefined) {
    dimensions.push({
      Icon: Gauge,
      name: 'Effort',
      // A level a newer build pinned has no label here; show it as stored.
      value: EFFORT_LABELS[effort] ?? effort,
      box: BOX,
      text: TEXT,
    });
  }
  if (dimensions.length === 0) return null;

  return {
    chips: dimensions.map(({ name, ...rest }) => ({
      ...rest,
      title: `${name} (planned): ${rest.value}`,
    })),
    label: `Planned assignment for ${subject}: ${dimensions
      .map((d) => `${d.name}: ${d.value}`)
      .join('; ')}`,
    tone: { chip: PLANNED_CHIP, icon: PLANNED_ICON },
  };
}
