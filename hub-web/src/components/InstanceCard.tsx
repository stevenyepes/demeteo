import { Laptop, Monitor } from 'lucide-react';

import { buttonClasses } from '../../../src/components/ui/Button';
import { Chip } from '../../../src/components/ui/Chip';
import type { FleetInstance } from '../lib/fleet';
import { elapsedText, instanceRows, platformText } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import type { RunSummary } from '../lib/runs';
import { InstanceRunRow } from './InstanceRunRow';
import { PresenceDot } from './PresenceDot';

interface InstanceCardProps {
  instance: FleetInstance;
  /** Its live runs; `null` when the run list could not be read, and the card falls back to the fleet's count. */
  runs: RunSummary[] | null;
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
}

export function InstanceCard({ instance, runs, now }: InstanceCardProps) {
  const name = instance.label ?? instance.id;
  const Icon = instance.os?.toLowerCase().startsWith('mac') ? Laptop : Monitor;
  const footer = [projectsText(instance.projectCount), seenText(instance, now)].filter((part) => part !== null);
  return (
    <article className="panel flex flex-col gap-4 p-5">
      <div className="flex items-start gap-3.5">
        <span
          aria-hidden="true"
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-hairline bg-white/5 text-slate-300"
        >
          <Icon className="h-5 w-5" />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <a
            href={routeHref({ name: 'instance', instanceId: instance.id })}
            className="font-heading text-lg font-semibold text-white break-all transition-colors hover:text-accent-ink"
          >
            {name}
          </a>
          <span className="font-mono text-xs text-slate-400 break-all">{platformText(instance)}</span>
        </div>
        <Chip tone={instance.present ? 'emerald' : 'slate'} icon={<PresenceDot present={instance.present} />}>
          {instance.present ? 'Online' : 'Offline'}
        </Chip>
      </div>
      <Body instance={instance} runs={runs} now={now} />
      <div className="flex flex-wrap items-center gap-2">
        {(instance.harnesses ?? []).map((harness) => (
          <Chip key={harness} tone="slate">
            {harness}
          </Chip>
        ))}
        {footer.length > 0 && (
          <span className="ml-auto font-mono text-xs text-slate-400">{footer.join(' · ')}</span>
        )}
      </div>
    </article>
  );
}

function Body({ instance, runs, now }: InstanceCardProps) {
  // `null` is withheld without `read`: "nothing running" would claim the machine is idle.
  if (instance.liveRuns === null || instance.needsYou === null) {
    return (
      <p className="nested-card px-4 py-3.5 text-sm text-slate-400">
        Read is off on this machine, so its runs and gates are not shown.
      </p>
    );
  }
  if (runs === null && instance.liveRuns > 0 && instance.needsYou.length === 0) {
    return (
      <p className="nested-card px-4 py-3.5 text-sm text-slate-400">
        {instance.liveRuns === 1 ? '1 live run' : `${instance.liveRuns} live runs`}
      </p>
    );
  }
  const rows = instanceRows(runs ?? [], instance.needsYou);
  if (rows.length > 0) {
    return (
      <ul aria-label={`Runs on ${instance.label ?? instance.id}`} className="flex flex-col gap-2">
        {rows.map((row) => (
          <li key={row.featureId}>
            <InstanceRunRow instanceId={instance.id} row={row} now={now} />
          </li>
        ))}
      </ul>
    );
  }
  return (
    <div className="nested-card flex items-center justify-between gap-3 px-4 py-3.5">
      <span className="text-sm text-slate-400">Idle — nothing running</span>
      {instance.present && (
        <a
          href={routeHref({ name: 'dispatch', instanceId: instance.id })}
          className={buttonClasses('secondary', 'sm')}
        >
          Run here
        </a>
      )}
    </div>
  );
}

function projectsText(count: number | null): string | null {
  if (count === null) return null;
  return count === 1 ? '1 project' : `${count} projects`;
}

function seenText(instance: FleetInstance, now: number): string {
  if (instance.present) return 'seen now';
  if (instance.lastSeen === null) return 'never seen';
  return `last seen ${elapsedText(instance.lastSeen, now)} ago`;
}
