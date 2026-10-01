import type {
  RunnerCompatibility,
  RunnerCompatibilityReport,
  RunnerIncompatibleError,
} from '../types';
import { asAppError } from './errors';
import { LOCAL_MACHINE } from './newDiscovery';
import { getRunnerCompatibility } from './runner';

/**
 * What the UI does with a machine's runner-compatibility verdict, kept out of
 * the components so the hook, the launch path and the notice all read one
 * policy. The verdict itself is computed in Rust
 * (`crates/demeteo-core/src/domain/runner_version.rs`) — nothing here compares
 * versions.
 */

const VERDICTS: ReadonlySet<RunnerCompatibility['verdict']> = new Set([
  'compatible',
  'runner_behind',
  'runner_ahead',
  'not_installed',
  'unknown',
]);

/** A detached submit refused by the backend's compatibility gate. `asAppError`
 *  keeps only `kind` and `message`, so the verdict is read off the raw value. */
export function isRunnerIncompatibleError(err: unknown): err is RunnerIncompatibleError {
  if (asAppError(err)?.kind !== 'runner_incompatible') return false;
  const { compatibility } = err as { compatibility?: unknown };
  if (compatibility == null || typeof compatibility !== 'object') return false;
  const { verdict } = compatibility as { verdict?: unknown };
  return typeof verdict === 'string' && VERDICTS.has(verdict as RunnerCompatibility['verdict']);
}

export interface RunnerNotice {
  /** `block` disables the launch; `info` is shown without stopping it. */
  tone: 'block' | 'info' | 'none';
  title: string;
  body: string;
}

const TITLES: Record<RunnerCompatibility['verdict'], string> = {
  compatible: '',
  runner_behind: 'Runner is older than Demeteo',
  runner_ahead: 'Runner is newer than Demeteo',
  not_installed: 'Runner not installed',
  unknown: "Couldn't verify the runner version",
};

/** `informational` is for surfaces that never call the runner (Ask and
 *  Discovery run over SSH, D1): a mismatch is worth knowing there, but stops
 *  nothing. */
export type RunnerNoticeVariant = 'blocking' | 'informational';

const SILENT: RunnerNotice = { tone: 'none', title: '', body: '' };

/** The body is the backend's own sentence, which already carries the upgrade
 *  direction and both channels when they differ. A missing runner only matters
 *  to a launch that needs one, so informational surfaces say nothing about it —
 *  otherwise every user who never runs detached is told to install it. */
export function noticeFor(
  report: RunnerCompatibilityReport,
  variant: RunnerNoticeVariant = 'blocking',
): RunnerNotice {
  if (report.verdict === 'compatible') return SILENT;
  if (report.verdict === 'not_installed' && variant === 'informational') return SILENT;
  return {
    tone: report.verdict === 'unknown' ? 'info' : 'block',
    title: TITLES[report.verdict],
    body: report.message,
  };
}

/** An unverifiable runner does not block: the submit gate re-checks it and
 *  refuses with its own error if the mismatch is real. */
export function blocksLaunch(report: RunnerCompatibilityReport | null | undefined): boolean {
  return report != null && noticeFor(report).tone === 'block';
}

/** The desktop host runs its own bundled binary, so there is nothing to probe. */
export function shouldProbe(machineId: string): boolean {
  return machineId !== '' && machineId !== LOCAL_MACHINE;
}

export const RUNNER_COMPATIBILITY_TTL_MS = 60_000;

interface CacheEntry {
  at: number;
  pending: Promise<RunnerCompatibilityReport>;
  report: RunnerCompatibilityReport | null;
}

const cache = new Map<string, CacheEntry>();

function freshEntry(machineId: string): CacheEntry | null {
  const entry = cache.get(machineId);
  if (!entry) return null;
  if (Date.now() - entry.at >= RUNNER_COMPATIBILITY_TTL_MS) {
    cache.delete(machineId);
    return null;
  }
  return entry;
}

/** The settled verdict for `machineId`, if one is still within the TTL. */
export function cachedRunnerCompatibility(machineId: string): RunnerCompatibilityReport | null {
  return freshEntry(machineId)?.report ?? null;
}

/** Concurrent callers share one probe; a failed probe is never cached. */
export function fetchRunnerCompatibility(
  machineId: string,
  { force = false }: { force?: boolean } = {},
): Promise<RunnerCompatibilityReport> {
  const existing = force ? null : freshEntry(machineId);
  if (existing) return existing.pending;

  const entry: CacheEntry = {
    at: Date.now(),
    pending: getRunnerCompatibility(machineId),
    report: null,
  };
  cache.set(machineId, entry);
  entry.pending.then(
    (report) => {
      entry.report = report;
    },
    () => {
      if (cache.get(machineId) === entry) cache.delete(machineId);
    },
  );
  return entry.pending;
}

/** Drop `machineId`'s verdict — or every machine's, with no argument — after
 *  anything that may have changed the installed runner. */
export function invalidateRunnerCompatibility(machineId?: string): void {
  if (machineId === undefined) cache.clear();
  else cache.delete(machineId);
}

/** Pushing this app's runner onto a newer one is a downgrade, so the Machines
 *  push action says so rather than calling it an upgrade (D9). */
export function pushActionLabel(
  report: RunnerCompatibilityReport | null | undefined,
  defaultLabel: string,
): string {
  return report?.verdict === 'runner_ahead' ? `Downgrade runner to ${report.app}` : defaultLabel;
}
