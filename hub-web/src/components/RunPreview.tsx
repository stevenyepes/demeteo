import { Lock } from 'lucide-react';
import { useId } from 'react';

import { Chip } from '../../../src/components/ui/Chip';
import { runStatusMeta } from '../../../src/lib/runStatus';
import { elapsedText } from '../lib/fleetSummary';
import type { HubScope } from '../lib/fleet';
import { routeHref } from '../lib/router';
import type { RunDetail } from '../lib/runs';
import { type ListedRun, ticketText } from '../lib/runsList';
import { stepTitles, ticketRows } from '../lib/runView';
import { useGateDecisions } from '../lib/useGateDecisions';
import { isNotFound, isScopeNotGranted, type StatusMap, useRunDetail } from '../lib/useRunDetail';
import { CeremonyError } from './CeremonyError';
import { GatePanel } from './GatePanel';
import { RequestStatus } from './RequestStatus';
import { RunEvents } from './RunEvents';
import { RunPipelineStrip } from './RunPipelineStrip';

interface RunPreviewProps {
  listed: ListedRun;
  /** The instance's granted scopes from the fleet read; `null` leaves a gate decision to the Hub. */
  scopes: HubScope[] | null;
  /** Unix seconds, injected so the relative times are testable. */
  now: number;
}

/**
 * The selected run beside the list: enough to decide its gate without
 * leaving it. Everything the mirror carries past that, the canvas and the
 * step inspector, stays on the run's own page.
 */
export function RunPreview({ listed, scopes, now }: RunPreviewProps) {
  const { run } = listed;
  const load = useRunDetail(run.instanceId, run.featureId);
  const headingId = useId();
  const title = load.state === 'loaded' ? (load.detail.title ?? run.title) : run.title;
  return (
    <div className="flex min-w-0 flex-col gap-4">
      <section aria-labelledby={headingId} className="panel flex flex-col gap-5 p-5 text-sm text-slate-400">
        <header className="flex flex-col gap-2">
          <PreviewChips listed={listed} now={now} />
          <h2 id={headingId} className="font-heading text-xl font-semibold text-white break-words">
            {title ?? <span className="font-mono">{run.featureId}</span>}
          </h2>
          <p className="flex flex-wrap gap-x-3 gap-y-1 font-mono text-xs">
            <a
              href={routeHref({ name: 'instance', instanceId: run.instanceId })}
              className="text-accent-ink transition-colors hover:text-white break-all"
            >
              {listed.instanceLabel}
            </a>
            {run.projectName !== null && <span className="break-all">{run.projectName}</span>}
            {load.state === 'loaded' && load.detail.workflow !== null && (
              <span className="break-all">{load.detail.workflow.name}</span>
            )}
            {title !== null && <span className="break-all">{run.featureId}</span>}
          </p>
        </header>
        {load.state === 'loading' && <p>Loading the run…</p>}
        {load.state === 'failed' && <PreviewFailure error={load.error} />}
        {load.state === 'loaded' && (
          <PreviewBody listed={listed} detail={load.detail} statusByNode={load.statusByNode} scopes={scopes} now={now} />
        )}
      </section>
      {load.state === 'loaded' && (
        <RunEvents events={load.detail.events} now={now} stepTitles={stepTitles(load.detail)} />
      )}
    </div>
  );
}

function PreviewChips({ listed, now }: { listed: ListedRun; now: number }) {
  const { run, gate } = listed;
  const status = gate !== null ? 'awaiting_gate' : run.status;
  const stepTitle = gate !== null ? (gate.stepTitle ?? run.stepTitle) : run.stepTitle;
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Chip status={status}>{runStatusMeta(status).label}</Chip>
      {stepTitle !== null && <Chip status="pending">{stepTitle}</Chip>}
      {gate !== null && <span className="font-mono text-xs">waiting {elapsedText(gate.since, now)}</span>}
    </div>
  );
}

function PreviewFailure({ error }: { error: unknown }) {
  if (isNotFound(error)) return <p>This run is no longer reported by its instance.</p>;
  if (isScopeNotGranted(error)) return <p>This instance does not grant read, so the run cannot be shown.</p>;
  return (
    <div className="flex flex-col gap-2">
      <p>The run could not be loaded.</p>
      <CeremonyError error={error} />
    </div>
  );
}

interface PreviewBodyProps {
  listed: ListedRun;
  detail: RunDetail;
  statusByNode: StatusMap;
  scopes: HubScope[] | null;
  now: number;
}

function PreviewBody({ listed, detail, statusByNode, scopes, now }: PreviewBodyProps) {
  const { run } = listed;
  const pipelineId = useId();
  const ticketsId = useId();
  const { gateDecisions, onGateDecided, onGateSettled } = useGateDecisions();
  const titles = stepTitles(detail);
  const tickets = ticketRows(detail.events);
  const gate = detail.pendingGates.find((pending) => pending.stepId !== null);
  const gateStepId = gate?.stepId ?? '';
  // Once the instance acts on a decision the mirror drops its gate, so the
  // latest decision stays shown with no gate left to show it under.
  const decision =
    gate === undefined ? Object.values(gateDecisions).pop() : gateDecisions[gate.stepExecutionId];
  const runHref = routeHref({ name: 'run', instanceId: run.instanceId, featureId: run.featureId });
  const facts = [
    { term: 'Tickets', value: ticketText(detail.tickets) ?? 'none reported' },
    { term: 'Step', value: run.stepTitle ?? run.stepId ?? 'none reported' },
    { term: 'Elapsed', value: run.startedAt === null ? 'not started' : elapsedText(run.startedAt, now) },
    ...(typeof run.costCents === 'number' ? [{ term: 'Cost', value: `$${(run.costCents / 100).toFixed(2)}` }] : []),
  ];
  return (
    <>
      <section aria-labelledby={pipelineId} className="nested-card flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <h3 id={pipelineId} className="text-xs font-medium text-slate-400">
            Pipeline
          </h3>
          <a href={runHref} className="text-xs text-accent-ink transition-colors hover:text-white">
            Open the run graph
          </a>
        </div>
        {detail.workflow === null ? (
          <p>This instance has not reported the run's graph.</p>
        ) : (
          <RunPipelineStrip workflow={detail.workflow} statusByNode={statusByNode} />
        )}
      </section>
      <dl aria-label="Run facts" className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-4">
        {facts.map(({ term, value }) => (
          <div key={term} className="flex flex-col gap-1">
            <dt className="text-xs">{term}</dt>
            <dd className="font-mono text-base text-slate-100 break-words">{value}</dd>
          </div>
        ))}
      </dl>
      {tickets.length > 0 && (
        <section aria-labelledby={ticketsId} className="nested-card flex flex-col p-4">
          <h3 id={ticketsId} className="pb-2 text-xs font-medium text-slate-400">
            Tickets
          </h3>
          <ul aria-label="Tickets" className="flex flex-col">
            {tickets.map((row) => (
              <li key={row.ticketId} className="flex items-baseline gap-3 border-t border-hairline py-2">
                <span className="font-mono text-slate-200 break-all">{row.ticketId}</span>
                <span className="ml-auto font-mono text-xs">{elapsedText(row.lastAt, now)} ago</span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <p className="nested-card flex items-center gap-3 px-4 py-3">
        <Lock aria-hidden="true" className="size-4 shrink-0" />
        <span>
          The diff and agent transcript stay on {listed.instanceLabel}. The Hub sees step and ticket events only.
        </span>
      </p>
      {gate !== undefined && gate.stepId !== null && (
        <div>
          <p className="text-slate-200">
            Waiting on you at <span className="font-medium">{titles[gate.stepId] ?? gate.stepId}</span>.
          </p>
          <GatePanel
            key={gate.stepExecutionId}
            instanceId={run.instanceId}
            featureId={run.featureId}
            stepExecutionId={gate.stepExecutionId}
            scopes={scopes}
            pending={decision !== undefined && !decision.settled}
            onDecided={(ticket) =>
              onGateDecided(gate.stepExecutionId, { stepId: gateStepId, ticket, settled: false })
            }
          />
        </div>
      )}
      {decision !== undefined && (
        <RequestStatus
          key={decision.ticket.requestId}
          instanceId={run.instanceId}
          ticket={decision.ticket}
          onSettled={() => onGateSettled(decision.ticket.requestId)}
        />
      )}
    </>
  );
}
