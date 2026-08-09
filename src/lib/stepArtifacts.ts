/**
 * Which of a step's declared artifacts a run surface lists, and how many it
 * folds away (UI_REDESIGN_PLAN §5.2).
 *
 * An `agent` step declares every file it wrote, so `artifact_paths` there is a
 * changed-files list rather than a set of documents. Thirty source paths in a
 * drill-down is not a drill-down: the markdown ones are what the agent wrote to
 * be *read*, and the rest are a diff, reviewable as one. Every other kind of
 * step declares only what it produced, so all of it lists.
 *
 * The count of what was folded is part of the answer rather than something the
 * caller re-derives — an agent step can produce nothing but source edits, and a
 * surface that only knows the listable paths would call that "no output".
 */
import { classifyArtifact } from './artifacts';
import type { StepExecution } from '../types';

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

  const listed = declared.filter((path) => classifyArtifact(path).kind === 'markdown');
  return { listed, hiddenCount: declared.length - listed.length };
}
