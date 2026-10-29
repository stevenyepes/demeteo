import { type ReactNode, useEffect, useRef, useState } from 'react';

import type { NodeRunStatus } from '../../../src/components/canvas/types';
import { WorkflowCanvas } from '../../../src/components/canvas/WorkflowCanvas';
import { runStatusMeta, TONE_TEXT } from '../../../src/lib/runStatus';
import { CeremonyError } from '../components/CeremonyError';
import { PlaceholderCard } from '../components/PlaceholderCard';
import { RunEvents } from '../components/RunEvents';
import { type GateDecisionTicket, StepInspector } from '../components/StepInspector';
import { TicketProgress } from '../components/TicketProgress';
import { HubApiError } from '../lib/api';
import { getInstance, type HubScope } from '../lib/fleet';
import { getRunDetail, type RunDetail } from '../lib/runs';
import { sameStatusMap, statusByNode } from '../lib/runView';

interface RunProps {
  instanceId: string;
  featureId: string;
}

type StatusMap = Record<string, NodeRunStatus>;

type Load =
  | { state: 'loading' }
  | {
      state: 'loaded';
      detail: RunDetail;
      statusByNode: StatusMap;
      /** Epoch ms of the read that answered `detail`. */
      readAt: number;
      /** The latest read after it failed, so `detail` is no longer live. */
      stale: boolean;
    }
  | { state: 'failed'; error: unknown };

const POLL_MS = 5000;

export function Run({ instanceId, featureId }: RunProps) {
  const [load, setLoad] = useState<Load>({ state: 'loading' });

  /** Bumped on unmount, so a read that settles after it has no page to land on. */
  const latestRead = useRef(0);

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let loaded = false;
    setLoad({ state: 'loading' });
    const poll = () => {
      getRunDetail(instanceId, featureId).then(
        (detail) => {
          if (latestRead.current !== read) return;
          loaded = true;
          setLoad((previous) => settled(previous, detail));
          if (detail.status === 'running') timer = setTimeout(poll, POLL_MS);
        },
        (error: unknown) => {
          if (latestRead.current !== read) return;
          // A poll that fails after a first load keeps the last answer on screen,
          // marked stale, and tries again; a lost session, a withdrawn run or a
          // revoked `read` would only fail the same way forever.
          if (loaded && !endsPolling(error)) {
            setLoad((previous) => (previous.state === 'loaded' ? { ...previous, stale: true } : previous));
            timer = setTimeout(poll, POLL_MS);
          } else {
            setLoad({ state: 'failed', error });
          }
        },
      );
    };
    poll();
    return () => {
      latestRead.current += 1;
      clearTimeout(timer);
    };
  }, [instanceId, featureId]);

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

  return (
    <PlaceholderCard title="Run">
      {load.state === 'loading' && <p>Loading run {ids}…</p>}
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
        <div className="flex max-w-xl flex-col gap-2">
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
          ids={ids}
        />
      )}
    </PlaceholderCard>
  );
}

/**
 * The canvas re-seeds its nodes whenever `definition` or `statusByNode`
 * changes identity, so a poll that reports nothing new hands back the objects
 * already on screen rather than equal copies of them.
 */
function settled(previous: Load, detail: RunDetail): Load {
  const next = statusByNode(detail.steps, detail.pendingGates);
  const readAt = Date.now();
  if (previous.state !== 'loaded') return { state: 'loaded', detail, statusByNode: next, readAt, stale: false };
  const held = previous.detail.workflow;
  const workflow =
    held !== null && detail.workflow !== null && JSON.stringify(held) === JSON.stringify(detail.workflow)
      ? held
      : detail.workflow;
  return {
    state: 'loaded',
    detail: { ...detail, workflow },
    statusByNode: sameStatusMap(previous.statusByNode, next) ? previous.statusByNode : next,
    readAt,
    stale: false,
  };
}

function isNotFound(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'not_found';
}

function isUnauthenticated(error: unknown): boolean {
  return error instanceof HubApiError && error.status === 401;
}

/** The Hub answers a revoked scope with `409`, never `403`. */
function isScopeNotGranted(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'scope_not_granted';
}

function endsPolling(error: unknown): boolean {
  return isUnauthenticated(error) || isNotFound(error) || isScopeNotGranted(error);
}

interface LoadedProps {
  instanceId: string;
  featureId: string;
  detail: RunDetail;
  statusByNode: StatusMap;
  readAt: number;
  stale: boolean;
  scopes: HubScope[] | null;
  ids: ReactNode;
}

function Loaded({ instanceId, featureId, detail, statusByNode, readAt, stale, scopes, ids }: LoadedProps) {
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  /** Here, not in the inspector, so a decision's status also survives closing and reopening it. */
  const [gateDecisions, setGateDecisions] = useState<Record<string, GateDecisionTicket>>({});
  const onGateDecided = (stepExecutionId: string, decided: GateDecisionTicket) => {
    // Re-inserted, so the step's latest decision is its last entry.
    setGateDecisions(({ [stepExecutionId]: _replaced, ...rest }) => ({ ...rest, [stepExecutionId]: decided }));
  };
  const onGateSettled = (requestId: string) => {
    setGateDecisions((held) => {
      const entry = Object.entries(held).find(([, decided]) => decided.ticket.requestId === requestId);
      if (entry === undefined || entry[1].settled) return held;
      return { ...held, [entry[0]]: { ...entry[1], settled: true } };
    });
  };
  const now = Math.floor(Date.now() / 1000);
  return (
    <div className="flex flex-col gap-3">
      <div className="nested-card flex flex-wrap items-center gap-3 px-4 py-3">
        <span>{ids}</span>
        <StatusLabel status={detail.status} />
      </div>
      {stale && <StaleNotice readAt={readAt} />}
      {detail.workflow === null ? (
        <NotReported detail={detail} statusByNode={statusByNode} onSelect={setSelectedStepId} />
      ) : (
        <>
          <div className="nested-card panel-field h-[32rem] w-full overflow-hidden">
            <WorkflowCanvas
              definition={detail.workflow}
              statusByNode={statusByNode}
              onNodeActivate={setSelectedStepId}
              selectedNodeId={selectedStepId}
            />
          </div>
          <OffGraphGates detail={detail} onSelect={setSelectedStepId} />
        </>
      )}
      {selectedStepId !== null && (
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
      <TicketProgress detail={detail} now={now} />
      <RunEvents events={detail.events} now={now} />
    </div>
  );
}

interface NotReportedProps {
  detail: RunDetail;
  statusByNode: StatusMap;
  onSelect: (stepId: string) => void;
}

/**
 * Never a graph drawn from `steps`: they carry no edges, so any layout of them would be invented.
 * A gate can name a step that has emitted no event yet, so it is listed after the reported ones.
 */
function NotReported({ detail, statusByNode, onSelect }: NotReportedProps) {
  const reported = new Set(detail.steps.map((step) => step.stepId));
  const gated = detail.pendingGates.flatMap((gate) =>
    gate.stepId === null || reported.has(gate.stepId) ? [] : [gate.stepId],
  );
  const listed = [...detail.steps.map((step) => step.stepId), ...new Set(gated)];
  return (
    <section className="nested-card px-4 py-3">
      <p className="text-slate-200">This instance has not reported the run's graph.</p>
      {listed.length === 0 ? (
        <p className="mt-2">No steps reported yet.</p>
      ) : (
        <ul aria-label="Reported steps" className="mt-2 flex flex-col gap-1">
          {listed.map((stepId) => (
            <li key={stepId}>
              <button
                type="button"
                onClick={() => onSelect(stepId)}
                className="flex w-full flex-wrap items-baseline gap-x-4 rounded text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
              >
                <span className="font-mono text-slate-200 break-all">{stepId}</span>
                <StatusLabel status={statusByNode[stepId]?.status ?? 'not reported'} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

interface OffGraphGatesProps {
  detail: RunDetail;
  onSelect: (stepId: string) => void;
}

/** A gate on a step the reported graph has no node for would otherwise have nothing on the canvas to open it from. */
function OffGraphGates({ detail, onSelect }: OffGraphGatesProps) {
  const nodes = new Set(detail.workflow?.nodes.map((node) => node.id));
  const offGraph = [
    ...new Set(
      detail.pendingGates.flatMap((gate) => (gate.stepId === null || nodes.has(gate.stepId) ? [] : [gate.stepId])),
    ),
  ];
  if (offGraph.length === 0) return null;
  return (
    <section className="nested-card px-4 py-3">
      <p className="text-slate-200">Gates pending on steps the graph does not show:</p>
      <ul aria-label="Gates off the graph" className="mt-2 flex flex-col gap-1">
        {offGraph.map((stepId) => (
          <li key={stepId}>
            <button
              type="button"
              onClick={() => onSelect(stepId)}
              className="flex w-full flex-wrap items-baseline gap-x-4 rounded text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
            >
              <span className="font-mono text-slate-200 break-all">{stepId}</span>
              <StatusLabel status="awaiting_gate" />
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function StaleNotice({ readAt }: { readAt: number }) {
  const at = new Date(readAt);
  return (
    <p
      role="status"
      className="nested-card max-w-xl px-3 py-2 text-sm text-slate-300"
    >
      Last updated at <time dateTime={at.toISOString()}>{at.toLocaleTimeString()}</time> — could not refresh.
    </p>
  );
}

function StatusLabel({ status }: { status: string }) {
  const meta = runStatusMeta(status);
  return <span className={`text-xs ${TONE_TEXT[meta.tone]}`}>{meta.label}</span>;
}
