import type { FleetInstance, NeedsYou } from './fleet';
import type { RunStatus, RunSummary, TicketProgress } from './runs';

export type FleetSummary = {
  instances: number;
  online: number;
  runsActive: number;
  gatesWaiting: number;
  withheld: number;
};

export type NeedsYouItem = NeedsYou & { instanceLabel: string };

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

const USERINFO = /^([a-z][a-z\d+.-]*:\/\/)[^/?#]*@/i;

export function summarizeFleet(fleet: FleetInstance[]): FleetSummary {
  const summary: FleetSummary = { instances: fleet.length, online: 0, runsActive: 0, gatesWaiting: 0, withheld: 0 };
  for (const instance of fleet) {
    if (instance.present) summary.online += 1;
    const runs = instance.liveRuns;
    const gates = instance.needsYou;
    // `null` is withheld without `read`: counting it as 0 would claim the instance is idle.
    if (runs === null || gates === null) summary.withheld += 1;
    if (runs !== null) summary.runsActive += runs;
    if (gates !== null) summary.gatesWaiting += gates.length;
  }
  return summary;
}

export function needsYouOldestFirst(fleet: FleetInstance[]): NeedsYouItem[] {
  return fleet
    .flatMap((instance) =>
      (instance.needsYou ?? []).map((item) => ({
        ...item,
        instanceId: instance.id,
        instanceLabel: instance.label ?? instance.id,
      })),
    )
    .sort((a, b) => a.since - b.since);
}

export function elapsedText(then: number | null, now: number): string {
  if (then === null) return 'never';
  const seconds = Math.max(0, Math.floor(now - then));
  if (seconds < MINUTE) return `${seconds}s`;
  if (seconds < HOUR) return `${Math.floor(seconds / MINUTE)}m`;
  if (seconds < DAY) return `${Math.floor(seconds / HOUR)}h`;
  return `${Math.floor(seconds / DAY)}d`;
}

export function platformText(parts: Pick<FleetInstance, 'os' | 'arch' | 'version'>): string {
  return [parts.os, parts.arch, parts.version].filter((part) => part !== null).join(' · ');
}

export function redactUserinfo(url: string): string {
  return url.replace(USERINFO, '$1');
}

export type FleetFilter = 'all' | 'online' | 'offline' | 'attention';

/** One row on an instance's card: a live run, a gate waiting on it, or both. */
export type InstanceRow = {
  featureId: string;
  title: string | null;
  projectName: string | null;
  step: string | null;
  tickets: TicketProgress | null;
  startedAt: number | null;
  status: RunStatus;
  gate: NeedsYou | null;
};

export function fleetSummaryText(summary: FleetSummary): string {
  return [
    counted(summary.instances, 'instance', 'instances'),
    `${summary.online} online`,
    counted(summary.runsActive, 'run active', 'runs active'),
    `${summary.gatesWaiting} need you`,
  ].join(' · ');
}

function counted(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

export function needsAttention(instance: FleetInstance): boolean {
  return (instance.needsYou?.length ?? 0) > 0;
}

export function fleetFilterCounts(fleet: FleetInstance[]): Record<FleetFilter, number> {
  return {
    all: fleet.length,
    online: fleet.filter((instance) => instance.present).length,
    offline: fleet.filter((instance) => !instance.present).length,
    attention: fleet.filter(needsAttention).length,
  };
}

/** Running runs only, grouped by instance, the longest-running first. */
export function liveRunsByInstance(runs: RunSummary[]): Map<string, RunSummary[]> {
  const grouped = new Map<string, RunSummary[]>();
  for (const run of runs) {
    if (run.status !== 'running') continue;
    const list = grouped.get(run.instanceId) ?? [];
    list.push(run);
    grouped.set(run.instanceId, list);
  }
  for (const list of grouped.values()) {
    list.sort((a, b) => (a.startedAt ?? a.updatedAt) - (b.startedAt ?? b.updatedAt));
  }
  return grouped;
}

/**
 * A gate is matched to its run by Feature. One with no live run beside it —
 * the run list was read before the gate parked, or not at all — still gets a
 * row of its own, since a card that hid a waiting gate would contradict the
 * Needs-you lane above it.
 */
export function instanceRows(runs: RunSummary[], gates: NeedsYou[]): InstanceRow[] {
  const gateOf = new Map(gates.map((gate) => [gate.featureId, gate]));
  const rows: InstanceRow[] = runs.map((run) => ({
    featureId: run.featureId,
    title: run.title,
    projectName: run.projectName,
    step: run.stepTitle ?? run.stepId,
    tickets: run.tickets,
    startedAt: run.startedAt,
    status: run.status,
    gate: gateOf.get(run.featureId) ?? null,
  }));
  const shown = new Set(runs.map((run) => run.featureId));
  for (const gate of gates) {
    if (shown.has(gate.featureId)) continue;
    rows.push({
      featureId: gate.featureId,
      title: gate.title,
      projectName: gate.projectName,
      step: gate.stepTitle ?? gate.stepId,
      tickets: null,
      startedAt: null,
      status: 'running',
      gate,
    });
  }
  return rows;
}

/**
 * Only what the Hub reports: how many tickets landed and failed. The plan's
 * total is not on the wire, so "3 of 5" would be a number nobody sent.
 */
export function ticketText(tickets: TicketProgress | null): string | null {
  if (tickets === null) return null;
  const { completed, failed, running } = tickets;
  if (completed + failed + running === 0) return null;
  const parts = [counted(completed, 'ticket landed', 'tickets landed')];
  if (failed > 0) parts.push(`${failed} failed`);
  return parts.join(', ');
}

export function filterFleet(
  fleet: FleetInstance[],
  runsOf: (instanceId: string) => RunSummary[],
  filter: FleetFilter,
  query: string,
): FleetInstance[] {
  const needle = query.trim().toLowerCase();
  return fleet.filter((instance) => {
    if (filter === 'online' && !instance.present) return false;
    if (filter === 'offline' && instance.present) return false;
    if (filter === 'attention' && !needsAttention(instance)) return false;
    if (needle === '') return true;
    const haystack = [
      instance.label,
      instance.id,
      instance.os,
      ...(instance.harnesses ?? []),
      ...runsOf(instance.id).map((run) => run.projectName),
      ...(instance.needsYou ?? []).map((gate) => gate.projectName),
    ];
    return haystack.some((value) => value?.toLowerCase().includes(needle) ?? false);
  });
}
