import { Chip } from '../../../src/components/ui/Chip';
import { runStatusMeta } from '../../../src/lib/runStatus';
import { elapsedText, type InstanceRow, ticketText } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import { ToneCard } from './ToneCard';

interface InstanceRunRowProps {
  instanceId: string;
  row: InstanceRow;
  /** Unix seconds, injected so the elapsed time is testable. */
  now: number;
}

export function InstanceRunRow({ instanceId, row, now }: InstanceRunRowProps) {
  const details = [
    row.projectName,
    row.step,
    row.gate === null ? ticketText(row.tickets) : `waiting ${elapsedText(row.gate.since, now)}`,
  ].filter((part) => part !== null);
  return (
    <ToneCard
      tone={row.gate === null ? runStatusMeta(row.status).tone : 'amber'}
      surface="nested"
      needsYou={row.gate !== null}
      className="flex flex-col gap-1 py-3 pr-3"
    >
      <div className="flex items-start justify-between gap-2">
        <a
          href={routeHref({ name: 'run', instanceId, featureId: row.featureId })}
          className={`min-w-0 text-sm font-medium text-slate-100 transition-colors hover:text-white ${
            row.title === null ? 'font-mono break-all' : 'break-words'
          }`}
        >
          {row.title ?? row.featureId}
        </a>
        {row.gate !== null ? (
          <Chip tone="amber">Gate needs you</Chip>
        ) : (
          row.startedAt !== null && (
            <span className="shrink-0 font-mono text-xs text-slate-300">{elapsedText(row.startedAt, now)}</span>
          )
        )}
      </div>
      {details.length > 0 && <span className="font-mono text-xs text-slate-400 break-all">{details.join(' · ')}</span>}
    </ToneCard>
  );
}
