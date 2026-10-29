import { Chip } from '../../../src/components/ui/Chip';
import { runStatusMeta } from '../../../src/lib/runStatus';
import { elapsedText } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import { type ListedRun, ticketText } from '../lib/runsList';
import { TONE_BAR } from './ToneCard';

interface RunRowProps {
  listed: ListedRun;
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
  /** The run whose preview is open beside the list. */
  selected: boolean;
}

/** Links to the list with this run's preview open; the preview links on to the run's own page. */
export function RunRow({ listed, now, selected }: RunRowProps) {
  const { run, gate, instanceLabel } = listed;
  const status = gate !== null ? 'awaiting_gate' : run.status;
  const meta = runStatusMeta(status);
  const facts = [
    instanceLabel,
    run.projectName,
    gate !== null ? (gate.stepTitle ?? run.stepTitle) : run.stepTitle,
    ticketText(run.tickets),
    timeText(listed, now),
    typeof run.costCents === 'number' ? `$${(run.costCents / 100).toFixed(2)}` : null,
  ].filter((fact): fact is string => fact !== null);
  return (
    <a
      href={routeHref({ name: 'runs', instanceId: run.instanceId, featureId: run.featureId })}
      aria-current={selected ? 'true' : undefined}
      className={`relative flex flex-col gap-1.5 overflow-hidden rounded-lg border py-3 pr-3 pl-5 transition-colors ${
        selected ? 'border-accent-edge bg-accent-tint' : 'border-transparent hover:border-hairline hover:bg-white/5'
      }`}
    >
      <span aria-hidden="true" className={`absolute inset-y-0 left-0 w-1 ${TONE_BAR[meta.tone]}`} />
      {run.title !== null ? (
        <span className="text-sm font-medium text-slate-100 break-words">{run.title}</span>
      ) : (
        <span className="font-mono text-sm text-slate-200 break-all">{run.featureId}</span>
      )}
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <Chip status={status}>{meta.label}</Chip>
        <span className="font-mono text-xs text-slate-400 break-words">{facts.join(' · ')}</span>
      </span>
    </a>
  );
}

function timeText({ run, gate }: ListedRun, now: number): string | null {
  if (gate !== null) return `waiting ${elapsedText(gate.since, now)}`;
  if (run.status === 'running') return run.startedAt === null ? null : elapsedText(run.startedAt, now);
  return `updated ${elapsedText(run.updatedAt, now)} ago`;
}
