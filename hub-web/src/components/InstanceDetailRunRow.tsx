import { Chip } from '../../../src/components/ui/Chip';
import { runStatusMeta } from '../../../src/lib/runStatus';
import { elapsedText } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import type { RunSummary, TicketProgress } from '../lib/runs';
import { ToneCard } from './ToneCard';

interface InstanceDetailRunRowProps {
  run: RunSummary;
  /** Unix second its gate has waited since; `null` when no gate on this run waits on a human. */
  gateSince: number | null;
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
}

/** One live run on the Instance page; the Fleet card's denser row is `InstanceRunRow`. */
export function InstanceDetailRunRow({ run, gateSince, now }: InstanceDetailRunRowProps) {
  const gated = gateSince !== null;
  const step = run.stepTitle ?? run.stepId;
  const tickets = ticketsText(run.tickets);
  return (
    <ToneCard tone={gated ? 'amber' : runStatusMeta(run.status).tone} surface="nested" needsYou={gated}>
      <a
        href={routeHref({ name: 'run', instanceId: run.instanceId, featureId: run.featureId })}
        className="flex flex-col gap-2 py-3 pr-4 transition-colors hover:bg-white/5"
      >
        <span className="flex flex-wrap items-start justify-between gap-x-3 gap-y-1">
          <span className="flex min-w-0 flex-col">
            {run.title !== null && (
              <span className="font-heading text-base font-semibold text-white break-words">{run.title}</span>
            )}
            <span
              className={`font-mono break-all ${run.title === null ? 'text-sm text-slate-200' : 'text-xs text-slate-400'}`}
            >
              {run.featureId}
            </span>
          </span>
          {gated ? <Chip status="awaiting_gate" dot /> : <Chip status={run.status} />}
        </span>
        <span className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-xs text-slate-400">
          {run.projectName !== null && <span className="break-all">{run.projectName}</span>}
          {step !== null && <span className="break-all">{tickets === null ? step : `${step} · ${tickets}`}</span>}
          {gated && <span>waiting {elapsedText(gateSince, now)}</span>}
          {run.startedAt !== null && <span>Started {elapsedText(run.startedAt, now)} ago</span>}
          <span>Updated {elapsedText(run.updatedAt, now)} ago</span>
          {typeof run.costCents === 'number' && <span>${(run.costCents / 100).toFixed(2)}</span>}
        </span>
      </a>
    </ToneCard>
  );
}

function ticketsText({ completed, failed, running }: TicketProgress): string | null {
  const parts: string[] = [];
  if (completed > 0) parts.push(completed === 1 ? '1 ticket landed' : `${completed} tickets landed`);
  if (running > 0) parts.push(`${running} running`);
  if (failed > 0) parts.push(`${failed} failed`);
  return parts.length === 0 ? null : parts.join(', ');
}
