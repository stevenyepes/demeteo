/**
 * Which of a step's declared artifacts a run surface lists, and how many it
 * folds away (UI_REDESIGN_PLAN §5.2).
 *
 * An `agent` step declares every file it wrote, so `artifact_paths` there is a
 * changed-files list rather than a set of documents. Thirty source paths in a
 * drill-down is not a drill-down: the ones the agent wrote to be *read* list,
 * and the rest are a diff, reviewable as one. Every other kind of step
 * declares only what it produced, so all of it lists.
 *
 * "Written to be read" is a document test, not a markdown test, and the
 * difference is load-bearing: `s-tickets` in the standard pipeline declares
 * exactly one artifact — `artifacts/task-list.json` — so a markdown-only rule
 * folds away the step's entire output and every surface built on this helper
 * then has no row for the ticket list at all. That included the gate picker
 * whose reason for existing is to reach it.
 *
 * The count of what was folded is part of the answer rather than something the
 * caller re-derives — an agent step can produce nothing but source edits, and a
 * surface that only knows the listable paths would call that "no output".
 */
import { classifyArtifact, type ArtifactKind } from './artifacts';
import type { StepExecution } from '../types';

const READABLE_KINDS: readonly ArtifactKind[] = ['markdown', 'task-list'];

export interface StepArtifacts {
  /** Paths to offer as openable rows, in declaration order. */
  listed: string[];
  /** Declared paths the rule above folded away. */
  hiddenCount: number;
}

const NONE: StepArtifacts = { listed: [], hiddenCount: 0 };

export function listStepArtifacts(step: StepExecution | null | undefined): StepArtifacts {
  if (!step) return NONE;

  // One local ref can be declared twice: a fetched remote artifact is cached
  // under its basename, so two runner paths ending the same way collapse onto
  // it. Counted once, and listed once — a repeat is a repeated React key.
  const declared = Array.from(
    new Set(
      step.artifact_paths?.length
        ? step.artifact_paths
        : step.artifact_path
          ? [step.artifact_path]
          : [],
    ),
  );

  if (step.step_kind !== 'agent') return { listed: declared, hiddenCount: 0 };

  const listed = declared.filter((path) => READABLE_KINDS.includes(classifyArtifact(path).kind));
  return { listed, hiddenCount: declared.length - listed.length };
}

/** One row-group per step strictly before the gate that has something
 *  listable, in step order. Shared by `GateArtifactPicker` (which renders the
 *  groups) and `GateView` (which chooses its empty-state copy from whether
 *  there are any) so the modal cannot tell the reviewer there is nothing to
 *  review while rows sit above the message saying so.
 *
 *  An earlier gate is excluded because it never produced anything: a gate step
 *  copies its predecessor's `artifact_paths` verbatim so its own card can show
 *  them (`steps/gate/mod.rs`), so listing it renders the producing step's
 *  artifacts a second time under a step that wrote none of them. Every shipped
 *  workflow has two gates, so the second one hits this. */
export function listReviewableGateArtifacts(
  steps: StepExecution[],
  gateStepIndex: number,
): { step: StepExecution; listed: string[] }[] {
  return steps
    .filter((step) => step.step_index < gateStepIndex && step.step_kind !== 'gate')
    .map((step) => ({ step, listed: listStepArtifacts(step).listed }))
    .filter(({ listed }) => listed.length > 0)
    .sort((a, b) => a.step.step_index - b.step.step_index);
}
