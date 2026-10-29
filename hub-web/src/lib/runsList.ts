import type { FleetInstance, NeedsYou } from './fleet';
import type { RunSummary, TicketProgress } from './runs';

export type RunsBand = 'needs-you' | 'active' | 'finished';

export type RunsShown = RunsBand | 'all';

/** A run the list shows, with what the fleet read adds to it. */
export type ListedRun = {
  run: RunSummary;
  band: RunsBand;
  /** The instance's label, or its id when it has none or the fleet has not been read. */
  instanceLabel: string;
  /** The oldest gate waiting on this run, `null` when none is. */
  gate: NeedsYou | null;
};

export type RunsBands = Record<RunsBand, ListedRun[]>;

function runKey(instanceId: string, featureId: string): string {
  return `${instanceId}/${featureId}`;
}

/**
 * The runs list reports no gate, so a waiting gate is read from the fleet: a
 * run that has one is still `running` there, and is banded as needing you.
 * Without the fleet, every run lands in its status's own band.
 */
export function bandRuns(runs: RunSummary[], fleet: FleetInstance[] | null): RunsBands {
  const labels = new Map<string, string>();
  const gates = new Map<string, NeedsYou>();
  for (const instance of fleet ?? []) {
    labels.set(instance.id, instance.label ?? instance.id);
    for (const gate of instance.needsYou ?? []) {
      const key = runKey(instance.id, gate.featureId);
      const held = gates.get(key);
      if (held === undefined || gate.since < held.since) gates.set(key, gate);
    }
  }
  const bands: RunsBands = { 'needs-you': [], active: [], finished: [] };
  for (const run of [...runs].sort((a, b) => b.updatedAt - a.updatedAt)) {
    const gate = gates.get(runKey(run.instanceId, run.featureId)) ?? null;
    const band: RunsBand = gate !== null ? 'needs-you' : run.status === 'running' ? 'active' : 'finished';
    bands[band].push({ run, band, instanceLabel: labels.get(run.instanceId) ?? run.instanceId, gate });
  }
  bands['needs-you'].sort((a, b) => (a.gate?.since ?? 0) - (b.gate?.since ?? 0));
  return bands;
}

/** `null` when no ticket has reported, so a run with no tickets says nothing rather than "0 of 0". */
export function ticketText(tickets: TicketProgress): string | null {
  const total = tickets.completed + tickets.failed + tickets.running;
  if (total === 0) return null;
  const done = `${tickets.completed} of ${total} tickets done`;
  return tickets.failed > 0 ? `${done}, ${tickets.failed} failed` : done;
}
