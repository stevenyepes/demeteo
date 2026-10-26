import { useId } from 'react';

import type { FleetInstance } from '../lib/fleet';

interface DispatchTargetListProps {
  instances: FleetInstance[];
  selectedId: string | null;
  onSelect: (instanceId: string) => void;
}

/**
 * Desktop instances only: no runner rows and no "unattended" toggle, because
 * runner dispatch is not built (docs/HUB.md §10) and a desktop run always
 * parks its gates for a human.
 *
 * Offline does not disable a row — a request to an absent instance queues
 * until it reconnects. Only a missing `spend` scope does, since the instance
 * would refuse the request whenever it arrived.
 */
export function DispatchTargetList({ instances, selectedId, onSelect }: DispatchTargetListProps) {
  const headingId = useId();

  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-2">
      <h2 id={headingId} className="font-heading text-sm font-semibold text-slate-200">
        Where to run
      </h2>
      {instances.length === 0 ? (
        <p className="text-sm text-slate-400">No instances are paired with this Hub yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {instances.map((instance) => (
            <li key={instance.id}>
              <TargetRow
                instance={instance}
                selected={instance.id === selectedId}
                onSelect={() => onSelect(instance.id)}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

interface TargetRowProps {
  instance: FleetInstance;
  selected: boolean;
  onSelect: () => void;
}

function TargetRow({ instance, selected, onSelect }: TargetRowProps) {
  const id = useId();
  const spend = instance.scopes.includes('spend');
  const describedBy = [`${id}-presence`, `${id}-scopes`, ...(spend ? [] : [`${id}-reason`])].join(' ');

  return (
    <button
      type="button"
      disabled={!spend}
      aria-pressed={selected}
      aria-labelledby={`${id}-name`}
      aria-describedby={describedBy}
      onClick={onSelect}
      className={`nested-card flex w-full flex-col gap-1.5 px-4 py-3 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400 disabled:cursor-not-allowed disabled:opacity-60 ${selected ? 'border-cyan-400/50 bg-cyan-500/10' : 'enabled:hover:border-cyan-400/30'}`}
    >
      <span className="flex flex-wrap items-center gap-3">
        <span id={`${id}-name`} className="font-mono text-sm text-white break-all">
          {instance.label ?? instance.id}
        </span>
        <span
          id={`${id}-presence`}
          className={`ml-auto flex items-center gap-2 rounded-full border px-2 py-0.5 text-xs font-medium ${instance.present ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-300' : 'border-slate-500/30 bg-slate-500/10 text-slate-400'}`}
        >
          <span
            aria-hidden="true"
            className={`inline-block h-2 w-2 shrink-0 rounded-full ${instance.present ? 'bg-emerald-400 animate-pulse-glow' : 'bg-slate-500'}`}
          />
          {instance.present ? 'online' : 'offline · requests queue until it reconnects'}
        </span>
      </span>
      <span id={`${id}-scopes`} className="font-mono text-xs text-slate-400">
        {instance.scopes.length === 0 ? 'No scopes granted' : `Scopes: ${instance.scopes.join(', ')}`}
      </span>
      {!spend && (
        <span id={`${id}-reason`} className="text-xs text-ruby-300">
          Spend has not been granted on this instance.
        </span>
      )}
    </button>
  );
}
