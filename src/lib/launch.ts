import { invoke } from "@tauri-apps/api/core";
import type { StagedAttachmentInput } from "./attachments";
import type { ErrorBus } from "./errorBus";
import type { EffortLevel, FeatureOrigin, LaunchedRun, StepOverride } from "../types";

/**
 * Mirrors the Rust `LaunchRunArgs`. A `null` or blank `machineId` launches on
 * the project's own compute; any other id is a detached run on that machine.
 * Which one it is gets decided in Rust (`domain::run_placement`), never here.
 *
 * The four detached-only fields — `targetRepoId`, `unattended`, `maxCostUsd`,
 * `maxWallClockSecs` — are refused on a local launch, so a caller sends them
 * only for a run it put on a machine. `targetRepoId` is singular because a
 * detached run clones exactly one repository.
 */
export interface LaunchRunArgs {
  machineId: string | null;
  projectId: string;
  workflowId: string;
  title: string;
  description: string;
  agentKind: string | null;
  model: string | null;
  effort: EffortLevel | null;
  commitArtifacts: boolean | null;
  loopIterations: number | null;
  maxBudgetUsd: number | null;
  stepOverrides: StepOverride[] | null;
  stagedAttachments: StagedAttachmentInput[] | null;
  targetRepoId: string | null;
  /** Omitted = a detached run's default, which is unattended. `false` is
   *  refused for a detached run. */
  unattended?: boolean;
  maxCostUsd: number | null;
  maxWallClockSecs: number | null;
  /** Left `undefined` — not `null` — when unstated, so JSON serialization
   *  drops the key and the run starts where every run started before the
   *  origin picker (migration V41). */
  origin?: FeatureOrigin;
  diffBaseBranch?: string;
}

/** Start a run wherever `args.machineId` places it. A detached run's shadow
 *  Feature is inserted before the runner RPC, so it is what comes back. */
export async function launchRun(args: LaunchRunArgs): Promise<LaunchedRun> {
  return invoke<LaunchedRun>("launch_run", { args });
}

/**
 * Tell the user what a launch that succeeded still owes them: one sticky
 * notice per note on `run`, none for a clean launch. Keyed off the notes
 * alone, never off where the run was placed, so a caller reports these
 * whichever transport ran it.
 *
 * Reported, never thrown: the run exists, and a launch that reads as failed
 * invites a second, paid submit of the same work.
 */
export function reportDegradedLaunch(
  run: LaunchedRun,
  reportError: ErrorBus["reportError"],
  openRuns: () => void,
): void {
  if (run.credentials_parked) {
    // `provider`: the undelivered credential is the git provider's PAT. Never
    // `transport`, whose toast offers Retry — retrying this launch submits a
    // second run.
    reportError(
      `"${run.title}" was accepted by the runner, but its git credentials could not be ` +
        `delivered (${run.credentials_parked}). It waits at Needs credentials until you ` +
        "re-inject them from Runs.",
      { kind: "provider", sticky: true, action: { label: "Open runs", onClick: openRuns } },
    );
  }
  if (run.mirror_unrecorded) {
    reportError(
      `"${run.title}" was accepted by the runner, but Demeteo could not record it ` +
        `(${run.mirror_unrecorded}), so Runs will not report on it. Check it on the machine.`,
      { kind: "database", sticky: true },
    );
  }
  if (run.ticket_unrecorded) {
    reportError(
      `"${run.title}" started, but its ticket could not record it (${run.ticket_unrecorded}). ` +
        "Do not start the ticket again — that would launch a second run.",
      { kind: "database", sticky: true },
    );
  }
}
