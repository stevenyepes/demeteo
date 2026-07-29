// HB7 — reading the harness verdict back out of what the engine persisted.
//
// Decision 44 made validate's verdict a *delta*: a gate red at the base commit
// and identically red now is not this feature's defect, so it is subtracted.
// That is worth nothing to a user who cannot see it happen — a subtraction
// nobody can audit is one nobody will trust the first time it is wrong.
//
// Three things reach a user through this file, and they are not the same thing:
//
//   1. a gate that was **already red at the base for an environmental reason**
//      (HB8/HB9) — the run stops at the baseline node, before the implement
//      budget;
//   2. a gate that **could not run at all** (exit 127, a timeout, a transport
//      fault) — terminal, with remediation;
//   3. a genuine **verdict** failure this feature caused — the rework loop's
//      job, and the only one of the three that is the feature's fault.
//
// # Where each side of the comparison comes from
//
// The *before* side is structured and authoritative: `features
// .harness_baseline_json` (V37) rides on the `Feature` row and reaches the
// frontend on `feature_get`, detached runs included.
//
// The *now* side is not stored per gate anywhere — `run_harness_first` folds it
// into the step's `error_message` and, on the all-excluded pass path, into the
// validate prompt (the report artifact is then the agent's own prose). So the
// parsers below read the two engine-authored shapes that *are* persisted:
// `build_environment_message` and `build_failure_reason` + `build_exclusion_note`.
// They are deliberately conservative — anything they cannot parse yields
// "nothing reported" rather than a guess, because the failure direction that
// matters is the same one decision 44 turns on: **absent must never read as
// green.**

import { ENVIRONMENT_ERROR_PREFIX } from './features';
import type { HarnessBaseline, HarnessBaselineRun, StepExecution } from '../types';

/** The engine's terminal "this machine cannot run the command" failure, split
 *  into the parts `build_environment_message` composed it from. */
export interface EnvironmentFailure {
  /** One sentence naming what the machine is missing. */
  reason: string;
  /** The concrete provisioning step. `''` when the classifier knew none — the
   *  panel says so rather than inventing one. */
  remediation: string;
  /** The command that could not run. */
  command: string;
  /** The machine it was asked to run on (`local`, or a machine id). */
  machine: string;
  /** The copy-pasteable reproduce block, newlines intact. */
  reproduce: string;
}

/** One gate the current run reported as failing, as named in the verdict. */
export interface FailingGate {
  name: string;
  command: string;
}

/** What the persisted step failure says about this run's gates. */
export interface HarnessEvidence {
  /** The step this was read off, so the UI can say where it came from. */
  stepId: string;
  /** Gates the verdict blamed on this feature. */
  failing: FailingGate[];
  /** Gate names the verdict explicitly excluded as pre-existing. */
  excluded: string[];
  /** Set when the step ended in the terminal environment failure. */
  environment: EnvironmentFailure | null;
}

/** The first line `build_environment_message` writes. Matching on the shared
 *  {@link ENVIRONMENT_ERROR_PREFIX} rather than the whole sentence keeps this
 *  wired to the same constant `isEnvironmentError` uses. */
const FAILING_COMMAND_LABEL = '\nFailing command: ';
const MACHINE_LABEL = '\nMachine: ';
const REPRODUCE_LABEL = '\nReproduce:\n';
const REMEDIATION_LABEL = '\nRemediation: ';

/**
 * Split the engine's terminal environment failure into its parts.
 *
 * `null` for anything that is not one — a verdict, an agent failure, an empty
 * message. The caller uses that to decide whether the step gets the
 * remediation-first presentation or the ordinary failure one, so guessing here
 * would dress a real defect up as somebody else's problem.
 *
 * The remediation is taken as everything between its label and the failing
 * command, because the 127 remediation is several lines long (it embeds a
 * `bash -l -i -c 'command -v …'` check) and a single-line read would truncate
 * exactly the part worth pasting.
 */
export function parseEnvironmentFailure(
  message: string | null | undefined,
): EnvironmentFailure | null {
  const raw = (message ?? '').trimStart();
  if (!raw.startsWith(ENVIRONMENT_ERROR_PREFIX)) return null;

  const cmdAt = raw.indexOf(FAILING_COMMAND_LABEL);
  // The head is everything the classifier said; the tail is the context the
  // orchestrator added. Without the tail the message is not the shape this
  // parses, so report nothing rather than half a card.
  if (cmdAt < 0) return null;

  const head = raw.slice(0, cmdAt);
  const tail = raw.slice(cmdAt);

  const remediationAt = head.indexOf(REMEDIATION_LABEL);
  const reasonBlock = remediationAt < 0 ? head : head.slice(0, remediationAt);
  const remediation =
    remediationAt < 0 ? '' : head.slice(remediationAt + REMEDIATION_LABEL.length).trim();

  // Drop the headline sentence: it is the same on every one of these, and the
  // panel renders its own heading.
  const reason = reasonBlock.split('\n').slice(1).join('\n').trim();

  const machineAt = tail.indexOf(MACHINE_LABEL);
  const command = tail
    .slice(FAILING_COMMAND_LABEL.length, machineAt < 0 ? undefined : machineAt)
    .trim();

  const reproduceAt = tail.indexOf(REPRODUCE_LABEL);
  const machine =
    machineAt < 0
      ? ''
      : tail
          .slice(machineAt + MACHINE_LABEL.length, reproduceAt < 0 ? undefined : reproduceAt)
          .trim();
  const reproduce =
    reproduceAt < 0 ? '' : tail.slice(reproduceAt + REPRODUCE_LABEL.length).trimEnd();

  return { reason, remediation, command, machine, reproduce };
}

/** `'lint' — command 'npm run lint' exited with failure:` — one block per red
 *  gate in `build_failure_reason`. The command capture is greedy so a command
 *  containing an apostrophe still ends at the right quote. */
const FAILING_GATE_RE = /^'([^']+)' — command '(.*)' exited with failure:$/gm;

/** The tail `build_exclusion_note` appends to a verdict reason. */
const EXCLUSION_NOTE_RE = /Also red, but NOT part of this verdict: ([^.]+)\./;

/**
 * Read the gates a verdict named, and the ones it named as *not* its own.
 *
 * Both halves matter and they are opposite claims: the first is what the rework
 * loop was told to fix, the second is what it was told to leave alone. A UI
 * that renders only the first shows a user a red gate in the log with no
 * explanation of why nothing is being done about it.
 */
export function parseHarnessVerdict(message: string | null | undefined): {
  failing: FailingGate[];
  excluded: string[];
} {
  const raw = message ?? '';
  const failing: FailingGate[] = [];
  for (const m of raw.matchAll(FAILING_GATE_RE)) {
    failing.push({ name: m[1], command: m[2] });
  }
  const note = EXCLUSION_NOTE_RE.exec(raw);
  const excluded = note
    ? Array.from(note[1].matchAll(/'([^']+)'/g)).map(m => m[1])
    : [];
  return { failing, excluded };
}

/**
 * The harness evidence this run left behind, taken from the **last** step that
 * carries any.
 *
 * Last rather than first because a rework loop re-runs validate: the earlier
 * attempt's verdict describes code that has since been changed, and showing it
 * beside a current baseline would invite exactly the misattribution decision 44
 * removes. `null` when no step said anything about a gate — which is the normal
 * state of a healthy run, and must not be confused with "the gates passed".
 */
export function readHarnessEvidence(steps: StepExecution[]): HarnessEvidence | null {
  for (let i = steps.length - 1; i >= 0; i -= 1) {
    const step = steps[i];
    const environment = parseEnvironmentFailure(step.error_message);
    const { failing, excluded } = parseHarnessVerdict(step.error_message);
    if (environment || failing.length > 0 || excluded.length > 0) {
      return { stepId: step.step_id, failing, excluded, environment };
    }
  }
  return null;
}

/** What the gate said at the base commit. `not-measured` is a real answer and
 *  the most important one: it is *not* `passed`. */
export type GateBaselineStatus = 'passed' | 'failed' | 'unrunnable' | 'not-measured';

/** What this run said about the gate. `not-reported` means no persisted step
 *  named it — again, not a pass. */
export type GateNowStatus = 'failed' | 'excluded' | 'unrunnable' | 'not-reported';

/** One row of the baseline-vs-now table. */
export interface GateRow {
  name: string;
  /** The command, preferring the one the *baseline* recorded, since that is the
   *  string the comparison was actually made against. */
  command: string;
  baseline: GateBaselineStatus;
  /** The classifier's sentence, when the baseline says the gate could not run. */
  baselineReason: string | null;
  /** Unix seconds; `null` when the gate was never measured. */
  measuredAt: number | null;
  producer: HarnessBaselineRun['producer'] | null;
  now: GateNowStatus;
}

function baselineStatus(run: HarnessBaselineRun): GateBaselineStatus {
  if (run.exit_ok) return 'passed';
  // Only a *positive* classification is an unrunnable gate. A record that was
  // never classified, one the classifier called a regression, and one written
  // before the field existed all decode the same way — the same fail-safe
  // direction `compare_gate` reads the field under, so a malfunctioning
  // classifier can never manufacture an escalation, here or there.
  return run.environment ? 'unrunnable' : 'failed';
}

/**
 * Join the baseline against this run, one row per gate.
 *
 * Row order is the baseline's own — the declared gate order, cheap gates first
 * — with any gate only the verdict knows about appended. A gate the baseline
 * never measured renders `not-measured`; nothing here ever fills that silence
 * with a pass.
 */
export function buildGateRows(
  baseline: HarnessBaseline | null | undefined,
  evidence: HarnessEvidence | null,
): GateRow[] {
  const measured = baseline?.harnesses ?? [];
  const failing = new Map(evidence?.failing.map(f => [f.name, f]) ?? []);
  const excluded = new Set(evidence?.excluded ?? []);
  const unrunnableCommand = evidence?.environment?.command ?? null;

  const nowStatus = (name: string, command: string): GateNowStatus => {
    // The terminal environment failure names a command, not a gate — it is
    // built from a reproduce line — so the command string is the join key.
    if (unrunnableCommand !== null && command !== '' && command === unrunnableCommand) {
      return 'unrunnable';
    }
    if (failing.has(name)) return 'failed';
    if (excluded.has(name)) return 'excluded';
    return 'not-reported';
  };

  const rows: GateRow[] = measured.map(run => ({
    name: run.name,
    command: run.command,
    baseline: baselineStatus(run),
    baselineReason: run.environment?.reason ?? null,
    measuredAt: run.measured_at,
    producer: run.producer,
    now: nowStatus(run.name, run.command),
  }));

  const seen = new Set(rows.map(r => r.name));
  for (const gate of evidence?.failing ?? []) {
    if (seen.has(gate.name)) continue;
    seen.add(gate.name);
    rows.push({
      name: gate.name,
      command: gate.command,
      baseline: 'not-measured',
      baselineReason: null,
      measuredAt: null,
      producer: null,
      now: nowStatus(gate.name, gate.command),
    });
  }
  return rows;
}

/**
 * Whether a terminal environment failure is one the **baseline** already found,
 * or one that only appeared during the run.
 *
 * They read identically in the message and they are different events. The first
 * stopped the run at the head of the graph with no implement budget spent
 * (HB9); the second stopped it wherever it happened to be. Only the record can
 * tell them apart, and only on a *positive* classification — an absent one is
 * never read as unrunnable, matching `unrunnable_baseline_gate`.
 */
export function isBaselineEnvironmentFailure(
  failure: EnvironmentFailure,
  baseline: HarnessBaseline | null | undefined,
): boolean {
  return (baseline?.harnesses ?? []).some(
    run => !run.exit_ok && run.environment != null && run.command === failure.command,
  );
}

/** First 12 characters of a sha — enough to identify a commit, short enough to
 *  read inline. Mirrors the engine's own `short_sha`. */
export function shortSha(sha: string): string {
  return sha.slice(0, 12);
}

/** Narrow an untyped `feature_get` payload to its baseline record.
 *
 *  A guard rather than a cast because the column is JSON written by the engine
 *  and read here across an IPC boundary that types nothing: a shape this build
 *  does not understand must degrade to "no baseline" — today's behaviour — the
 *  same way `HarnessBaseline::from_column` degrades every decode failure to
 *  `None`. Inventing a record is the one direction that is not survivable. */
export function readHarnessBaseline(feature: unknown): HarnessBaseline | null {
  if (typeof feature !== 'object' || feature === null) return null;
  const candidate = (feature as { harness_baseline?: unknown }).harness_baseline;
  if (typeof candidate !== 'object' || candidate === null) return null;
  const record = candidate as { base_sha?: unknown; harnesses?: unknown };
  if (typeof record.base_sha !== 'string') return null;
  const harnesses = Array.isArray(record.harnesses)
    ? record.harnesses.filter(isBaselineRun)
    : [];
  return { base_sha: record.base_sha, harnesses };
}

function isBaselineRun(value: unknown): value is HarnessBaselineRun {
  if (typeof value !== 'object' || value === null) return false;
  const run = value as Record<string, unknown>;
  return (
    typeof run.name === 'string' &&
    typeof run.command === 'string' &&
    typeof run.exit_ok === 'boolean' &&
    typeof run.measured_at === 'number' &&
    (run.producer === 'node' || run.producer === 'fallback')
  );
}
