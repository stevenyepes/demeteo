import type { FleetInstance } from '../lib/fleet';
import { elapsedText, platformText } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import { PresenceDot } from './PresenceDot';

interface InstanceCardProps {
  instance: FleetInstance;
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
}

export function InstanceCard({ instance, now }: InstanceCardProps) {
  const gates = instance.needsYou?.length ?? 0;
  return (
    <article
      className={`nested-card flex flex-col gap-2 px-4 py-3 ${gates > 0 ? 'border-amber-500/30 bg-amber-500/10' : ''}`}
    >
      <div className="flex flex-wrap items-center gap-3">
        <a
          href={routeHref({ name: 'instance', instanceId: instance.id })}
          className="font-heading font-semibold text-white break-all transition-colors hover:text-cyan-300 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
        >
          {instance.label ?? instance.id}
        </a>
        <span className="ml-auto flex items-center gap-2 text-xs">
          <PresenceDot present={instance.present} />
          <span aria-hidden="true" className={instance.present ? 'text-emerald-300' : 'text-slate-400'}>
            {instance.present ? 'Online' : 'Offline'}
          </span>
        </span>
      </div>
      <span className="font-mono text-xs text-slate-400 break-all">{platformText(instance)}</span>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-400">
        <span>{liveRunsText(instance.liveRuns)}</span>
        <span>{instance.lastSeen === null ? 'Never seen' : `Last seen ${elapsedText(instance.lastSeen, now)} ago`}</span>
        {gates > 0 && (
          <span className="font-medium text-amber-500">
            {gates === 1 ? '1 gate needs you' : `${gates} gates need you`}
          </span>
        )}
      </div>
    </article>
  );
}

function liveRunsText(liveRuns: number | null): string {
  // `null` is withheld without `read`; "0" would claim the machine is idle.
  if (liveRuns === null) return 'Read is off on this machine';
  return liveRuns === 1 ? '1 live run' : `${liveRuns} live runs`;
}
