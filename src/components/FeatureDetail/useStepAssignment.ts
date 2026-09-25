import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import type { EffortLevel } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import { remoteSetStepAssignment, setStepAssignment } from '../../lib/features';
import type { RemoteRunMirror, StepOverride } from '../../types';
import type { HarnessOverrides } from './useHarnessOverrides';

export interface StepAssignment {
  /** Whether the picker now says something other than what the node is
   *  pinned to — i.e. whether Apply has anything to write. */
  dirty: boolean;
  /** Whether the node has a stored pin — the stored row, not the picker, so
   *  blanking the controls by hand does not make Reset read as "already
   *  inheriting" while the pin still stands. */
  pinned: boolean;
  /** A submit is in flight *against the selected node*. Apply and Reset share
   *  it: they are the same write with different payloads, and both are
   *  one-at-a-time. */
  applying: boolean;
  /**
   * The backend's own text for the last submit that did not land — a refused
   * status, a repository write error, or an older runner's
   * `unknown method: set_step_assignment`.
   *
   * It is state rather than a dialog because the claim it corrects is on
   * screen: the control goes on showing the trio the user chose, and a
   * dismissed dialog leaves that reading as "pinned". `useRerunActions` can
   * use a dialog because a failed retry leaves nothing behind that lies.
   */
  error: string | null;
  /** Stable. */
  clearError: () => void;
  /** Pin the picker's current trio on the selected node. Stable. */
  apply: () => Promise<void>;
  /** Drop the node's pin and return it to the resolution chain. Stable. */
  reset: () => Promise<void>;
}

export interface StepAssignmentInput {
  /** The picker this assignment reads. One instance is shared with the
   *  rerun controls — a second would probe the machine twice and drift. */
  overrides: HarnessOverrides;
  /**
   * The DAG node id, which is what `StepOverride.step_id` is keyed by, and
   * `selectedExecutionId` is the `step_executions` row the command names.
   * One node needs both: the pin outlives every execution of that node, so
   * neither id can stand in for the other.
   */
  selectedStepId: string | null;
  selectedExecutionId: string | null;
  /** The run's tier-1 pins (`Feature.step_overrides`). */
  stepOverrides?: StepOverride[];
  /** Non-null for a run the runner owns, which routes the write to it. */
  remoteRun: RemoteRunMirror | null;
  /** Re-read the run so the applied pin becomes the new seed. */
  reload: () => void;
}

/**
 * Apply and Reset for the one node the inspector is on.
 *
 * **One node, one pin.** A write here never touches a descendant and there is
 * no cone toggle: the blast radius of the control is exactly what it names.
 * "This node and everything after it" is expressible — one tier-1 entry per
 * descendant, off the replay cone — but it is a different control with a
 * different confirmation, not a checkbox on this one.
 *
 * **The submitted trio is the whole stored row, never a patch over it.** All
 * three dimensions go every time, seeded from the existing pin, which is what
 * makes Reset a plain all-`null` submit. Under merge semantics, clearing one
 * dimension would need a third state the picker has no way to spell.
 *
 * **Both pieces of submit state belong to one node, and neither outlives it.**
 * The inspector is shared, so the next click swaps the node under a control
 * that kept rendering — and `error`'s whole warrant for being state is that
 * the claim it corrects is on screen. A refusal naming a node the user has
 * left is not a stale banner, it is a false one: under the next node's picker
 * it reads as a statement about that node, and under a running node's
 * read-only chip it reads as that node's explanation. `applying` fails the
 * same way with the buttons instead of the words. So both reset on retarget,
 * and a submit that outlives its node lands its outcome nowhere.
 *
 * **Every member of the returned object is identity-stable across a render
 * that changed none of them, and so is the object** — `useHarnessOverrides`
 * records why that matters on this surface. The writers read their inputs
 * through a ref for the same reason: `reload` and `overrides` are rebuilt by
 * their own hooks every render, so naming them as dependencies would stabilize
 * nothing.
 */
export function useStepAssignment(input: StepAssignmentInput): StepAssignment {
  const { overrides, selectedStepId, stepOverrides } = input;
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const latest = useRef(input);
  latest.current = input;

  const pin = selectedStepId === null
    ? undefined
    : stepOverrides?.find((o) => o.step_id === selectedStepId);
  const pinned = pin !== undefined;
  // Effort against what the seed wrote, not the raw pin: a pin whose effort
  // the pinned harness no longer declares (hermes, whose ladder is empty by
  // decision) is seeded as "inherit", and comparing to the stored level would
  // light Apply up on a node nobody touched. Nor a reconcile of our own: the
  // seed used the catalog as it stood then, and a second reading against a
  // catalog that landed since disagrees with it — which also holds Retry and
  // Replay shut on the node.
  const dirty =
    overrides.selectedAgent !== (pin?.agent_kind ?? '') ||
    overrides.selectedModel !== (pin?.model ?? '') ||
    overrides.selectedEffort !== overrides.seededEffort;

  useEffect(() => {
    setApplying(false);
    setError(null);
  }, [selectedStepId]);

  const submit = useCallback(async (assignment: {
    agentKind: string | null;
    model: string | null;
    effort: EffortLevel | null;
  }) => {
    const { selectedStepId: target, selectedExecutionId, remoteRun, reload } = latest.current;
    setError(null);
    if (!selectedExecutionId) {
      setError('This node has no execution in this run yet, so there is nothing to assign it on.');
      return false;
    }
    setApplying(true);
    // Read after every await, never captured: the retarget may land either
    // side of the response, and the effect above only covers the side where
    // it lands first.
    const onTarget = () => latest.current.selectedStepId === target;
    try {
      if (remoteRun) {
        await remoteSetStepAssignment({
          machineId: remoteRun.machine_id,
          runId: remoteRun.run_id,
          stepExecutionId: selectedExecutionId,
          ...assignment,
        });
      } else {
        await setStepAssignment({ stepExecutionId: selectedExecutionId, ...assignment });
      }
      // The write landed on `target` whether or not the user is still looking
      // at it, and the run is read as a whole, so this is not conditional.
      reload();
      return onTarget();
    } catch (err) {
      if (onTarget()) setError(formatError(err));
      return false;
    } finally {
      if (onTarget()) setApplying(false);
    }
  }, []);

  const apply = useCallback(async () => {
    const { overrides: picker } = latest.current;
    await submit({
      agentKind: picker.selectedAgent || null,
      model: picker.selectedModel || null,
      effort: picker.selectedEffort || null,
    });
  }, [submit]);

  const reset = useCallback(async () => {
    if (!(await submit({ agentKind: null, model: null, effort: null }))) return;
    // Only once it landed, and the reload that re-seeds from the now-absent
    // pin is a poll away: until then the picker would still read as the trio
    // it just un-pinned. Clearing on a refusal would say the opposite of what
    // `error` says.
    const { overrides: picker } = latest.current;
    picker.onAgentChange('');
    picker.setSelectedModel('');
    picker.setSelectedEffort('');
  }, [submit]);

  const clearError = useCallback(() => setError(null), []);

  return useMemo(
    () => ({ dirty, pinned, applying, error, clearError, apply, reset }),
    [dirty, pinned, applying, error, clearError, apply, reset],
  );
}
