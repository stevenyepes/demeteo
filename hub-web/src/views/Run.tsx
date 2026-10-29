import { useEffect, useState } from 'react';

import { WorkflowCanvas } from '../../../src/components/canvas/WorkflowCanvas';
import { Chip } from '../../../src/components/ui/Chip';
import { runStatusMeta } from '../../../src/lib/runStatus';
import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { RunEvents } from '../components/RunEvents';
import { RunGatesWaiting } from '../components/RunGatesWaiting';
import { RunStepList } from '../components/RunStepList';
import { StepInspector } from '../components/StepInspector';
import { TicketProgress } from '../components/TicketProgress';
import { getInstance, type HubScope } from '../lib/fleet';
import { useFleet } from '../lib/fleetContext';
import { routeHref } from '../lib/router';
import type { RunDetail } from '../lib/runs';
import { useGateDecisions } from '../lib/useGateDecisions';
import { stepTitles } from '../lib/runView';
import { isNotFound, isScopeNotGranted, type StatusMap, useRunDetail } from '../lib/useRunDetail';

interface RunProps {
  instanceId: string;
  featureId: string;
}

export function Run({ instanceId, featureId }: RunProps) {
  const load = useRunDetail(instanceId, featureId);

  const [scopes, setScopes] = useState<HubScope[] | null>(null);

  useEffect(() => {
    let current = true;
    setScopes(null);
    getInstance(instanceId).then(
      (instance) => {
        if (current) setScopes(instance.scopes);
      },
      () => {},
    );
    return () => {
      current = false;
    };
  }, [instanceId]);

  const ids = (
    <>
      <span className="font-mono text-slate-200 break-all">{featureId}</span> on instance{' '}
      <span className="font-mono text-slate-200 break-all">{instanceId}</span>
    </>
  );

  const loaded = load.state === 'loaded' ? load.detail : null;

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title={loaded === null ? 'Run' : (loaded.title ?? featureId)}
        eyebrow={loaded === null ? undefined : <RunBreadcrumb instanceId={instanceId} detail={loaded} />}
        summary={loaded === null ? undefined : <RunSummaryLine featureId={featureId} detail={loaded} />}
      />
      {load.state === 'loading' && <p className="text-sm text-slate-400">Loading run {ids}…</p>}
      {load.state === 'failed' && isNotFound(load.error) && (
        <p
          role="alert"
          className="max-w-xl rounded-lg border border-ruby-500/30 bg-ruby-500/10 px-3 py-2 text-sm text-ruby-200 break-words"
        >
          Run {ids} was not found or not yours.
        </p>
      )}
      {load.state === 'failed' && isScopeNotGranted(load.error) && (
        <p
          role="alert"
          className="max-w-xl rounded-lg border border-ruby-500/30 bg-ruby-500/10 px-3 py-2 text-sm text-ruby-200 break-words"
        >
          Instance <span className="font-mono break-all">{instanceId}</span> does not grant read, so run{' '}
          <span className="font-mono break-all">{featureId}</span> cannot be shown.
        </p>
      )}
      {load.state === 'failed' && !isNotFound(load.error) && !isScopeNotGranted(load.error) && (
        <div className="flex max-w-xl flex-col gap-2 text-sm text-slate-400">
          <p>Run {ids} could not be loaded.</p>
          <CeremonyError error={load.error} />
        </div>
      )}
      {load.state === 'loaded' && (
        <Loaded
          instanceId={instanceId}
          featureId={featureId}
          detail={load.detail}
          statusByNode={load.statusByNode}
          readAt={load.readAt}
          stale={load.stale}
          scopes={scopes}
        />
      )}
    </div>
  );
}

interface LoadedProps {
  instanceId: string;
  featureId: string;
  detail: RunDetail;
  statusByNode: StatusMap;
  readAt: number;
  stale: boolean;
  scopes: HubScope[] | null;
}

function Loaded({ instanceId, featureId, detail, statusByNode, readAt, stale, scopes }: LoadedProps) {
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  /** Here, not in the inspector, so a decision's status also survives closing and reopening it. */
  const { gateDecisions, onGateDecided, onGateSettled } = useGateDecisions();
  const now = Math.floor(Date.now() / 1000);
  const titles = stepTitles(detail);
  return (
    <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_26rem]">
      <div className="flex min-w-0 flex-col gap-4">
        {stale && <StaleNotice readAt={readAt} />}
        {detail.workflow === null ? (
          <NotReported detail={detail} statusByNode={statusByNode} titles={titles} onSelect={setSelectedStepId} />
        ) : (
          <>
            <section aria-label="Pipeline graph" className="panel panel-field h-[36rem] w-full overflow-hidden">
              <WorkflowCanvas
                definition={detail.workflow}
                statusByNode={statusByNode}
                onNodeActivate={setSelectedStepId}
                selectedNodeId={selectedStepId}
              />
            </section>
            <OffGraphGates detail={detail} titles={titles} onSelect={setSelectedStepId} />
          </>
        )}
        <TicketProgress detail={detail} now={now} />
      </div>
      <aside aria-label="Run inspector" className="flex min-w-0 flex-col gap-4">
        {selectedStepId === null ? (
          <RunGatesWaiting detail={detail} titles={titles} now={now} onSelect={setSelectedStepId} />
        ) : (
          <StepInspector
            instanceId={instanceId}
            featureId={featureId}
            stepId={selectedStepId}
            detail={detail}
            scopes={scopes}
            gateDecisions={gateDecisions}
            onGateDecided={onGateDecided}
            onGateSettled={onGateSettled}
            onClose={() => setSelectedStepId(null)}
          />
        )}
        <RunEvents events={detail.events} now={now} stepTitles={titles} />
        <p className="text-xs text-slate-400">
          Transcripts, diffs and agent output stay on the instance. The Hub sees step and ticket events only.
        </p>
      </aside>
    </div>
  );
}

function RunBreadcrumb({ instanceId, detail }: { instanceId: string; detail: RunDetail }) {
  const { fleet } = useFleet();
  const label = fleet?.find((instance) => instance.id === instanceId)?.label ?? instanceId;
  return (
    <span className="flex flex-wrap items-center gap-2">
      <a href={routeHref({ name: 'runs' })} className="text-accent-ink transition-colors hover:text-white">
        Runs
      </a>
      <span aria-hidden="true">/</span>
      <a
        href={routeHref({ name: 'instance', instanceId })}
        className="text-accent-ink transition-colors hover:text-white break-all"
      >
        {label}
      </a>
      {detail.projectName !== null && (
        <>
          <span aria-hidden="true">/</span>
          <span className="break-all">{detail.projectName}</span>
        </>
      )}
    </span>
  );
}

function RunSummaryLine({ featureId, detail }: { featureId: string; detail: RunDetail }) {
  const status = detail.status === 'running' && detail.pendingGates.length > 0 ? 'awaiting_gate' : detail.status;
  return (
    <span className="flex flex-wrap items-center gap-3">
      <Chip status={status}>{runStatusMeta(status).label}</Chip>
      <span className="break-all">{featureId}</span>
      {detail.workflow !== null && <span className="break-all">{detail.workflow.name}</span>}
    </span>
  );
}

interface NotReportedProps {
  detail: RunDetail;
  statusByNode: StatusMap;
  titles: Record<string, string>;
  onSelect: (stepId: string) => void;
}

/**
 * Never a graph drawn from `steps`: they carry no edges, so any layout of them would be invented.
 * A gate can name a step that has emitted no event yet, so it is listed after the reported ones.
 */
function NotReported({ detail, statusByNode, titles, onSelect }: NotReportedProps) {
  const reported = new Set(detail.steps.map((step) => step.stepId));
  const gated = detail.pendingGates.flatMap((gate) =>
    gate.stepId === null || reported.has(gate.stepId) ? [] : [gate.stepId],
  );
  const listed = [...detail.steps.map((step) => step.stepId), ...new Set(gated)];
  return (
    <section className="panel flex flex-col gap-3 p-5 text-sm text-slate-400">
      <p className="text-slate-200">This instance has not reported the run's graph.</p>
      {listed.length === 0 ? (
        <p>No steps reported yet.</p>
      ) : (
        <RunStepList
          label="Reported steps"
          steps={listed.map((stepId) => ({
            stepId,
            title: titles[stepId] ?? null,
            status: statusByNode[stepId]?.status ?? 'not reported',
          }))}
          onSelect={onSelect}
        />
      )}
    </section>
  );
}

interface OffGraphGatesProps {
  detail: RunDetail;
  titles: Record<string, string>;
  onSelect: (stepId: string) => void;
}

/** A gate on a step the reported graph has no node for would otherwise have nothing on the canvas to open it from. */
function OffGraphGates({ detail, titles, onSelect }: OffGraphGatesProps) {
  const nodes = new Set(detail.workflow?.nodes.map((node) => node.id));
  const offGraph = [
    ...new Set(
      detail.pendingGates.flatMap((gate) => (gate.stepId === null || nodes.has(gate.stepId) ? [] : [gate.stepId])),
    ),
  ];
  if (offGraph.length === 0) return null;
  return (
    <section className="panel flex flex-col gap-3 p-5 text-sm">
      <p className="text-slate-200">Gates pending on steps the graph does not show:</p>
      <RunStepList
        label="Gates off the graph"
        steps={offGraph.map((stepId) => ({ stepId, title: titles[stepId] ?? null, status: 'awaiting_gate' }))}
        onSelect={onSelect}
      />
    </section>
  );
}

function StaleNotice({ readAt }: { readAt: number }) {
  const at = new Date(readAt);
  return (
    <p role="status" className="nested-card px-3 py-2 text-sm text-slate-300">
      Last updated at <time dateTime={at.toISOString()}>{at.toLocaleTimeString()}</time> — could not refresh.
    </p>
  );
}
