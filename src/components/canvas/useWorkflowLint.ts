/**
 * Live structural lint for the builder (task P3.3): debounced round-trips to
 * the `workflow_lint` command as the author edits.
 *
 * Two deliberate behaviors:
 *
 * - **The previous result stays visible while a new one is in flight.** Lint
 *   badges that blink off on every keystroke read as flicker, and the stale
 *   answer is the *right* answer for all but the last edit.
 * - **Out-of-order replies are dropped.** Each request carries a sequence
 *   number and only a reply newer than the one already rendered is applied, so
 *   a slow early lint can't overwrite a fast later one.
 *
 * The definition is serialized once per change and used as the debounce key,
 * so a re-render that produces an identical graph (very common — the canvas
 * re-derives its React Flow arrays constantly) costs no IPC.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { EMPTY_LINT, indexFindings, type LintFinding, type LintIndex } from './lint';
import type { WorkflowDefinitionV2 } from './types';

/** Long enough that typing in the config panel doesn't spam the backend,
 *  short enough that a fixed error clears while the author is still looking. */
export const LINT_DEBOUNCE_MS = 300;

export interface WorkflowLintState {
  lint: LintIndex;
  /** A lint is in flight (the `lint` value may be one edit stale). */
  checking: boolean;
  /** The command itself failed (not a finding — an IPC/backend problem). */
  error: string | null;
}

export function useWorkflowLint(
  definition: WorkflowDefinitionV2 | null,
  debounceMs: number = LINT_DEBOUNCE_MS,
): WorkflowLintState {
  const [lint, setLint] = useState<LintIndex>(EMPTY_LINT);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Serialize once: the debounce key *and* the payload.
  const payload = useMemo(
    () => (definition ? JSON.stringify(definition) : null),
    [definition],
  );

  const seq = useRef(0);
  const rendered = useRef(0);

  useEffect(() => {
    if (payload === null) {
      setLint(EMPTY_LINT);
      setChecking(false);
      return;
    }

    setChecking(true);
    const mine = (seq.current += 1);
    const timer = setTimeout(() => {
      invoke<LintFinding[]>('workflow_lint', { definition: JSON.parse(payload) })
        .then((findings) => {
          if (mine < rendered.current) return; // a newer lint already landed
          rendered.current = mine;
          setLint(indexFindings(findings));
          setError(null);
          setChecking(false);
        })
        .catch((err) => {
          if (mine < rendered.current) return;
          rendered.current = mine;
          setError(String(err));
          setChecking(false);
        });
    }, debounceMs);

    return () => clearTimeout(timer);
  }, [payload, debounceMs]);

  return { lint, checking, error };
}
