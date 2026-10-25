import type { FleetInstance, NeedsYou } from './fleet';

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
