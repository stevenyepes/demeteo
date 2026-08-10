import type { ReactNode } from 'react';
import { AlertTriangle, Check, HelpCircle, MinusCircle, XCircle } from 'lucide-react';
import type { GateBaselineStatus, GateNowStatus, GateRow, HarnessEvidence } from '../lib/harnessVerdict';
import { buildGateRows, shortSha } from '../lib/harnessVerdict';
import type { HarnessBaseline } from '../types';

// HB7 — validate's verdict, per gate, as a table the user can audit.
//
// Decision 44 subtracts a failure nobody asked to have subtracted: a gate red
// at the base commit and identically red now is not this feature's defect, so
// it does not fail the step. That is only trustworthy if it is visible. A
// subtraction the user cannot see is indistinguishable from a bug the first
// time it *is* one, and the whole value of the mechanism is that validate stops
// blaming the feature for failures it did not cause.
//
// The one property every branch below protects: **absent is not green.** A gate
// with no baseline renders "not measured" — never a pass, never nothing at all
// — because reading the first as the second is precisely the inversion decision
// 44 exists to prevent.

/** How one status renders: label, tone classes, and the icon that carries it
 *  for anyone not distinguishing the tone. */
interface Chip {
  label: string;
  className: string;
  icon: ReactNode;
  title: string;
}

/** Tones per AGENTS.md §4 — emerald healthy, ruby failure, amber the machine's
 *  problem rather than the code's. "Not measured" and "excluded" get no
 *  semantic colour on purpose: neither is a health claim, and painting them
 *  would be the UI answering a question the record did not. */
const NEUTRAL = 'bg-white/5 border-white/10 text-slate-300';
const EMERALD = 'bg-emerald-500/10 border-emerald-500/20 text-emerald-400';
const RUBY = 'bg-ruby-500/10 border-ruby-500/20 text-ruby-400';
const AMBER = 'bg-amber-500/10 border-amber-500/20 text-amber-400';

const ICON = 'w-3 h-3 shrink-0';

function baselineChip(status: GateBaselineStatus): Chip {
  switch (status) {
    case 'passed':
      return {
        label: 'passed',
        className: EMERALD,
        icon: <Check className={ICON} />,
        title: 'This gate exited zero against the base commit, before the feature started.',
      };
    case 'failed':
      return {
        label: 'already failing',
        className: RUBY,
        icon: <XCircle className={ICON} />,
        title:
          'This gate was already red at the base commit. The failure predates the feature, so an identical failure now is excluded from the verdict.',
      };
    case 'unrunnable':
      return {
        label: 'could not run here',
        className: AMBER,
        icon: <AlertTriangle className={ICON} />,
        title:
          'The gate was red at the base because this machine cannot run it — it reached no verdict, so there is nothing to subtract and nothing to blame the feature for.',
      };
    case 'not-measured':
      return {
        label: 'not measured',
        className: NEUTRAL,
        icon: <HelpCircle className={ICON} />,
        title:
          'Nothing measured this gate at the base commit. That is an absence of evidence, not a pass — a failure here cannot be told apart from a pre-existing one.',
      };
  }
}

function nowChip(status: GateNowStatus): Chip {
  switch (status) {
    case 'failed':
      return {
        label: 'failed — this feature',
        className: RUBY,
        icon: <XCircle className={ICON} />,
        title: 'This gate failed and the baseline does not excuse it, so it counts against the feature.',
      };
    case 'excluded':
      return {
        label: 'excluded — pre-existing',
        className: NEUTRAL,
        icon: <MinusCircle className={ICON} />,
        title:
          'This gate failed, identically to the base commit, so it was subtracted from the verdict rather than blamed on the feature.',
      };
    case 'unrunnable':
      return {
        label: 'could not run here',
        className: AMBER,
        icon: <AlertTriangle className={ICON} />,
        title: 'The command never ran on this machine, so it proved nothing about the feature.',
      };
    case 'not-reported':
      return {
        label: 'no failure reported',
        className: NEUTRAL,
        icon: <MinusCircle className={ICON} />,
        title:
          'No step reported this gate as failing. The engine records a gate only when it fails, so this is not a recorded pass.',
      };
  }
}

const PRODUCER_LABEL: Record<NonNullable<GateRow['producer']>, string> = {
  node: 'measured at the head of this run, before any work started',
  fallback: 'measured on the failure path against a fresh checkout of the base commit',
};

function StatusChip({ chip }: { chip: Chip }) {
  return (
    <span
      title={chip.title}
      className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded border text-[10px] font-mono whitespace-nowrap ${chip.className}`}
    >
      {chip.icon}
      {chip.label}
    </span>
  );
}

/**
 * The audit sentence for a gate the baseline excuses.
 *
 * Rendered for the gate the verdict explicitly excluded *and* for a gate that
 * was red at the base but which no step has named — because on the all-excluded
 * pass path the engine writes the exclusion into the validate prompt and never
 * into a step row, so the record is the only evidence the user has. The two
 * wordings differ: one states what happened, the other states what would.
 */
function ExclusionNote({ row, baseSha }: { row: GateRow; baseSha: string }) {
  const provenance = row.producer ? ` (${PRODUCER_LABEL[row.producer]})` : '';
  const sha = baseSha === '' ? 'the base commit' : `base commit ${shortSha(baseSha)}`;
  const text =
    row.now === 'excluded'
      ? `Excluded from this step's verdict: '${row.name}' failed with the identical output at ${sha}${provenance}. The failure predates this feature, so the rework loop was told not to fix it.`
      : `'${row.name}' was already failing at ${sha}${provenance}. A failure here is excluded from the verdict rather than blamed on this feature.`;
  return (
    <p className="mt-1.5 text-[11px] leading-relaxed text-slate-400 font-sans">{text}</p>
  );
}

interface Props {
  /** The V37 record for this run, or `null` when none was measured. */
  baseline: HarnessBaseline | null;
  /** What this run's persisted step failures say about the same gates. */
  evidence: HarnessEvidence | null;
}

/**
 * Baseline vs. now, per gate.
 *
 * Renders nothing at all when there is neither a baseline nor a reported gate:
 * an empty table on every healthy run would be noise, and the panel is only
 * honest when it has something to be honest about.
 */
export function HarnessGateTable({ baseline, evidence }: Props) {
  const rows = buildGateRows(baseline, evidence);
  if (rows.length === 0) return null;

  const baseSha = baseline?.base_sha ?? '';

  return (
    <section
      data-testid="harness-gate-table"
      className="glass-panel mb-6 w-full shrink-0 border border-white/10 p-5"
    >
      <header className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="font-heading text-sm font-semibold tracking-wide text-white">
          Harness gates — before this feature vs. now
        </h3>
        {baseline ? (
          <span className="font-mono text-[10px] text-slate-500">
            baseline at {shortSha(baseSha)}
          </span>
        ) : (
          <span
            data-testid="harness-no-baseline"
            title="Nothing measured this project's gates at the base commit, so no failure can be attributed or excused. This is not a claim that the gates were green."
            className="inline-flex items-center gap-1.5 rounded border border-white/10 bg-white/5 px-2 py-0.5 font-mono text-[10px] text-slate-300"
          >
            <HelpCircle className={ICON} />
            no baseline measured
          </span>
        )}
      </header>
      <p className="mt-1 font-sans text-[11px] leading-relaxed text-slate-400">
        Validate judges a <span className="text-slate-200">delta</span>: a gate that was already
        failing before this feature is excluded from the verdict instead of being blamed on it.
        A gate with no measurement is <span className="text-slate-200">unknown</span>, not passing.
      </p>

      {/* One block per gate rather than a three-column table. The panel's seat
          is the run's meta track, which is a fraction of the window and narrow
          at every window the app opens in — the table carried a `min-w-[36rem]`
          that its own track could not honour, so the third column was clipped
          and the escape hatch was a horizontal scrollbar inside a side panel.
          Blocks read the same comparison at any width and cannot clip; the
          before/now pair keeps its own two-column grid, which is the part that
          is genuinely tabular. */}
      <ul className="mt-4 space-y-2.5">
        {rows.map(row => (
          <li
            key={row.name}
            data-gate-row={row.name}
            className="rounded-lg border border-white/5 bg-white/[0.02] p-3"
          >
            <div className="font-mono text-xs text-slate-200">{row.name}</div>
            <div className="mt-0.5 font-mono text-[10px] text-slate-500 break-all">
              {row.command}
            </div>
            <div className="mt-2.5 grid grid-cols-2 gap-3">
              <div className="min-w-0">
                <div className="mb-1 font-sans text-[10px] font-bold uppercase tracking-wider text-slate-500">
                  At the base commit
                </div>
                <StatusChip chip={baselineChip(row.baseline)} />
                {row.baselineReason && (
                  <p className="mt-1.5 font-sans text-[11px] leading-relaxed text-amber-400/80">
                    {row.baselineReason}
                  </p>
                )}
              </div>
              <div className="min-w-0">
                <div className="mb-1 font-sans text-[10px] font-bold uppercase tracking-wider text-slate-500">
                  This run
                </div>
                <StatusChip chip={nowChip(row.now)} />
                {row.baseline === 'failed' &&
                  (row.now === 'excluded' || row.now === 'not-reported') && (
                    <ExclusionNote row={row} baseSha={baseSha} />
                  )}
              </div>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
