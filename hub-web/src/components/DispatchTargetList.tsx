import { Monitor } from 'lucide-react';
import { useId } from 'react';

import { Chip } from '../../../src/components/ui/Chip';
import type { FleetInstance } from '../lib/fleet';
import { platformText } from '../lib/fleetSummary';

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
    <section aria-labelledby={headingId} className="flex flex-col gap-3">
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
  const platform = platformText(instance);

  return (
    <button
      type="button"
      disabled={!spend}
      aria-pressed={selected}
      aria-labelledby={`${id}-name`}
      aria-describedby={describedBy}
      onClick={onSelect}
      className={`nested-card flex w-full items-start gap-3 px-3 py-3 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${
        selected ? 'border-accent-edge bg-accent-tint' : 'enabled:hover:bg-white/5'
      }`}
    >
      <span
        aria-hidden="true"
        className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-hairline bg-white/5 ${
          selected ? 'text-white' : 'text-slate-400'
        }`}
      >
        <Monitor className="h-4 w-4" />
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-1.5">
        <span className="flex flex-wrap items-center gap-2">
          <span id={`${id}-name`} className="font-heading text-sm font-semibold text-white break-all">
            {instance.label ?? instance.id}
          </span>
          <span id={`${id}-presence`} className="ml-auto">
            <Chip tone={instance.present ? 'emerald' : 'slate'} dot>
              {instance.present ? 'online' : 'offline · requests queue until it reconnects'}
            </Chip>
          </span>
        </span>
        {platform !== '' && <span className="font-mono text-xs text-slate-400 break-all">{platform}</span>}
        {instance.harnesses !== null && instance.harnesses.length > 0 && (
          <span className="flex flex-wrap gap-1.5">
            {instance.harnesses.map((harness) => (
              <Chip key={harness} tone="slate">
                {harness}
              </Chip>
            ))}
          </span>
        )}
        <span id={`${id}-scopes`} className="font-mono text-xs text-slate-400">
          {instance.scopes.length === 0 ? 'No scopes granted' : `Scopes: ${instance.scopes.join(', ')}`}
        </span>
        {!spend && (
          <span id={`${id}-reason`} className="text-xs text-ruby-300">
            Spend has not been granted on this instance.
          </span>
        )}
      </span>
    </button>
  );
}
