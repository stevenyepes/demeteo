/**
 * `NodePanel` — the node drill-down side panel (tasks P2.3 + P2.4, PRD §6.2).
 *
 * Clicking a node on the run-mode `WorkflowCanvas` opens this panel beside the
 * graph (the same split-panel shape `ArtifactViewer` uses in the timeline). It
 * answers J2 ("which node, which attempt, what did it cost, why did it fail")
 * for the selected node from data Phase-1 now persists, across four tabs:
 *
 *  - **Overview** — status, failure class, the per-attempt table from
 *    `step_attempts` (class · cost · duration · applied rule), so a retry loop
 *    is legible instead of collapsed onto one row.
 *  - **Live** — the running node's `agent_stream` transcript buffer (P2.4).
 *  - **Output** — the node's declared artifacts (Monaco via `ArtifactViewer`)
 *    plus the harness/verifier output (`error_message`).
 *  - **Actions** — Retry / Replay-from-node / Stop / Decide-gate (P2.4), all
 *    respecting the ancestor guard with disabled-button explanations. The panel
 *    holds no run logic of its own: FeatureDetail owns the handlers and passes
 *    them in, so the canvas and timeline drive the exact same code paths.
 */
import { Fragment, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  AlertCircle,
  Check,
  CircleDashed,
  Cpu,
  Loader2,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  X,
  XCircle,
} from 'lucide-react';

import { ArtifactViewer } from '../ArtifactViewer';
import { RunEventFeed } from '../RunEventFeed';
import {
  ArtifactIcon,
  ARTIFACT_KIND_COLORS,
  ARTIFACT_KIND_LABELS,
  classifyArtifact,
} from '../../lib/artifacts';
import { formatError } from '../../lib/errors';
import { formatDuration } from '../../lib/utils';
import { runStatusMeta, TONE_CHIP, TONE_TEXT } from '../../lib/runStatus';
import type { RunEvent, SequenceState, StepAttempt, StepExecution } from '../../types';
import { nodeTypeMeta, type NodeConfigV2, type NodeRunStatus } from './types';

/** Human label for a failure class (`error_class` / retry-policy key). */
const CLASS_LABELS: Record<string, string> = {
  environment: 'Environment',
  verdict: 'Verdict',
  agent_failure: 'Agent failure',
  non_retryable: 'Non-retryable',
};

function classLabel(cls: string): string {
  return CLASS_LABELS[cls] ?? cls.replace(/_/g, ' ');
}

/** ms → the shared seconds-based duration formatter. */
function formatMs(ms: number | null | undefined): string {
  if (ms == null) return '—';
  return formatDuration(ms / 1000);
}

function formatCost(cost: number | null | undefined): string {
  if (cost == null) return '—';
  return `$${cost.toFixed(cost < 1 ? 4 : 2)}`;
}

/** The active ancestor blocking a manual retry/gate decision, if any. */
export interface BlockingAncestor {
  step_id: string;
  status: string;
}

export interface NodePanelProps {
  featureId: string;
  /** The selected graph node (from the pinned migrated definition). */
  node: NodeConfigV2;
  /** Live overlay state for this node, if the run has reached it. */
  run: NodeRunStatus | null;
  /** The `step_executions` row backing the node — artifacts + error output. */
  step: StepExecution | null;
  onClose: () => void;
  /** Open a worktree-ref artifact in the code editor (passed to `ArtifactViewer`). */
  onOpenEditorForPath?: (filePath: string) => void;
  /** Delegate an artifact click to the host's shared artifact overlay
   *  (`ArtifactModal`) instead of previewing it inline. When passed, the panel
   *  renders no `ArtifactViewer` of its own, so the graph drill-down and the
   *  timeline open the same surface. Omitted = today's inline preview. */
  onOpenArtifact?: (artifactPath: string) => void;
  /** The run's unified `run_events` feed (P1.13) — local push or remote poll,
   *  same shape either way (P2.6). Rendered raw in the Overview tab, replacing
   *  the standalone `RunEventTimeline` as a separate surface. */
  runEvents?: RunEvent[];

  // --- P2.4 ---
  /** Live `agent_stream` buffer for the backing execution (running nodes). */
  liveStream?: string;
  /** True while the node is running/verifying — drives the Live tab affordance. */
  isStreaming?: boolean;
  /** A non-terminal ancestor that blocks retry/gate decisions, else null. */
  blockedBy?: BlockingAncestor | null;
  /** Re-run this node from scratch (`step_retry`). Absent = not offered. */
  onRetry?: () => void;
  /** Replay from this node (`replay_from_step`); opens the confirm modal and
   *  highlights the downstream cone on the canvas. */
  onReplay?: () => void;
  /** Stop the running execution. */
  onStop?: () => void;
  /** Open the full-screen `GateView` for an awaiting gate node. */
  onDecideGate?: () => void;
}

type Tab = 'overview' | 'live' | 'output' | 'actions';

export function NodePanel({
  featureId,
  node,
  run,
  step,
  onClose,
  onOpenEditorForPath,
  onOpenArtifact,
  runEvents,
  liveStream,
  isStreaming,
  blockedBy,
  onRetry,
  onReplay,
  onStop,
  onDecideGate,
}: NodePanelProps) {
  const [tab, setTab] = useState<Tab>('overview');
  const meta = nodeTypeMeta(node.type);
  const TypeIcon = meta.icon;

  const status = run?.status ?? 'pending';
  const statusMeta = runStatusMeta(status);
  const errorClass = run?.errorClass ?? null;
  const stepExecutionId = run?.stepExecutionId ?? null;

  // Per-attempt history, fetched once at panel level and shared by the Overview
  // table and the Actions retry hint. Refetches when the backing execution
  // changes or advances (a new attempt closing moves status/cost).
  const [attempts, setAttempts] = useState<StepAttempt[]>([]);
  const [attemptsLoading, setAttemptsLoading] = useState(false);
  const [attemptsError, setAttemptsError] = useState<string | null>(null);
  const version = `${run?.status}:${run?.costUsd}:${run?.wallClockSecs}`;
  useEffect(() => {
    if (!stepExecutionId) {
      setAttempts([]);
      return;
    }
    let cancelled = false;
    setAttemptsLoading(true);
    setAttemptsError(null);
    invoke<StepAttempt[]>('step_attempts_list', { executionId: stepExecutionId })
      .then((rows) => {
        if (!cancelled) setAttempts(rows);
      })
      .catch((err) => {
        if (!cancelled) setAttemptsError(formatError(err) || 'Failed to load attempts.');
      })
      .finally(() => {
        if (!cancelled) setAttemptsLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stepExecutionId, version]);

  // Artifacts declared by this node's execution, deduped (two runner artifacts
  // sharing a basename can cache to one local ref — same rule the timeline uses).
  const artifactPaths = useMemo(() => {
    const raw = step?.artifact_paths?.length
      ? step.artifact_paths
      : step?.artifact_path
        ? [step.artifact_path]
        : [];
    return Array.from(new Set(raw));
  }, [step]);

  const [selectedArtifact, setSelectedArtifact] = useState<string | null>(null);
  // Reset the tab + artifact selection whenever the panel retargets a node.
  useEffect(() => {
    setTab('overview');
    setSelectedArtifact(null);
  }, [node.id]);

  const hasOutput = artifactPaths.length > 0 || !!step?.error_message;
  const hasActions = !!(onRetry || onReplay || onStop || onDecideGate);

  return (
    // Clamped, not a fixed percentage: at 4K a flat 62% is a ~2100px panel and
    // leaves the canvas 38% of the window, while in a half-width window the
    // same fraction is too narrow to read. A basis with a floor and a ceiling
    // gives the graph everything past the panel's comfortable reading width.
    <div className="flex h-full basis-[42%] min-w-[20rem] max-w-[38rem] shrink-0 flex-col border-l border-white/5 bg-[#0d0f14]/80 backdrop-blur-xl">
      {/* Header */}
      <div className="flex shrink-0 items-start justify-between gap-3 border-b border-white/5 px-5 py-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <TypeIcon className={`h-4 w-4 shrink-0 ${TONE_TEXT[meta.tone]}`} />
            <h3 className="truncate font-display text-sm font-bold uppercase tracking-wider text-white">
              {node.title}
            </h3>
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-2">
            <span
              className={`rounded border px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider ${TONE_CHIP[statusMeta.tone]}`}
            >
              {statusMeta.label}
            </span>
            <span className="text-[10px] font-mono uppercase tracking-wider text-slate-500">
              {meta.label}
            </span>
            {errorClass && (
              <span className="rounded border border-ruby-500/20 bg-ruby-500/10 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider text-ruby-400">
                {classLabel(errorClass)}
              </span>
            )}
          </div>
        </div>
        <button
          onClick={onClose}
          className="shrink-0 rounded-lg bg-white/5 p-1.5 text-slate-400 transition hover:bg-white/10 hover:text-white"
          title="Close panel"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Tabs */}
      <div className="flex shrink-0 gap-1 border-b border-white/5 px-3 pt-2">
        <TabButton active={tab === 'overview'} onClick={() => setTab('overview')}>
          Overview
        </TabButton>
        <TabButton active={tab === 'live'} onClick={() => setTab('live')}>
          Live
        </TabButton>
        <TabButton active={tab === 'output'} onClick={() => setTab('output')}>
          Output
        </TabButton>
        <TabButton active={tab === 'actions'} onClick={() => setTab('actions')}>
          Actions
        </TabButton>
      </div>

      {/* Body */}
      <div className="min-h-0 flex-1 overflow-hidden">
        {tab === 'overview' && (
          <OverviewTab
            run={run}
            hasExecution={!!stepExecutionId}
            attempts={attempts}
            loading={attemptsLoading}
            error={attemptsError}
            isSequence={node.type === 'sequence'}
            featureId={featureId}
            nodeId={node.id}
            stepExecutionId={stepExecutionId}
            version={version}
            runEvents={runEvents}
          />
        )}
        {tab === 'live' && <LiveTab liveStream={liveStream} isStreaming={!!isStreaming} />}
        {tab === 'output' && (
          <OutputTab
            step={step}
            hasOutput={hasOutput}
            artifactPaths={artifactPaths}
            selectedArtifact={selectedArtifact}
            onSelectArtifact={setSelectedArtifact}
            onOpenEditorForPath={onOpenEditorForPath}
            onOpenArtifact={onOpenArtifact}
          />
        )}
        {tab === 'actions' && (
          <ActionsTab
            node={node}
            run={run}
            hasActions={hasActions}
            attempts={attempts}
            blockedBy={blockedBy ?? null}
            onRetry={onRetry}
            onReplay={onReplay}
            onStop={onStop}
            onDecideGate={onDecideGate}
          />
        )}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-t-md px-3 py-1.5 text-xs font-semibold transition ${
        active
          ? 'border-b-2 border-cyan-400 text-cyan-300'
          : 'border-b-2 border-transparent text-slate-400 hover:text-slate-200'
      }`}
    >
      {children}
    </button>
  );
}

/** Overview: node totals + the per-attempt history table (`step_attempts`),
 *  plus — for a sequence node — its landed-vs-pending task list (P2.5). */
function OverviewTab({
  run,
  hasExecution,
  attempts,
  loading,
  error,
  isSequence,
  featureId,
  nodeId,
  stepExecutionId,
  version,
  runEvents,
}: {
  run: NodeRunStatus | null;
  hasExecution: boolean;
  attempts: StepAttempt[];
  loading: boolean;
  error: string | null;
  isSequence: boolean;
  featureId: string;
  nodeId: string;
  stepExecutionId: string | null;
  version: string;
  runEvents?: RunEvent[];
}) {
  return (
    <div className="h-full space-y-5 overflow-y-auto px-5 py-4">
      {/* Totals */}
      <div className="grid grid-cols-3 gap-3">
        <Stat label="Attempts" value={run ? String(Math.max(attempts.length, 1)) : '—'} />
        <Stat label="Total cost" value={formatCost(run?.costUsd)} />
        <Stat label="Duration" value={run?.wallClockSecs != null ? formatDuration(run.wallClockSecs) : '—'} />
      </div>

      {/* Sequence task list — the Decision-13 landed prefix, made legible. */}
      {isSequence && (
        <SequenceTasks
          featureId={featureId}
          nodeId={nodeId}
          stepExecutionId={stepExecutionId}
          version={version}
        />
      )}

      {/* Attempt table */}
      <div>
        <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
          Attempt history
        </div>
        {!hasExecution ? (
          <EmptyHint>This node hasn&apos;t started yet.</EmptyHint>
        ) : loading && attempts.length === 0 ? (
          <div className="flex items-center gap-2 py-6 text-xs text-slate-500">
            <Loader2 className="h-4 w-4 animate-spin text-violet-400" /> Loading attempts…
          </div>
        ) : error ? (
          <div className="flex items-start gap-2 rounded-lg border border-rose-500/20 bg-rose-950/20 p-3 text-xs text-rose-300">
            <AlertCircle className="mt-px h-4 w-4 shrink-0 text-rose-400" />
            <span>{error}</span>
          </div>
        ) : attempts.length === 0 ? (
          <EmptyHint>No attempt rows recorded.</EmptyHint>
        ) : (
          <div className="overflow-hidden rounded-xl border border-white/5">
            <table className="w-full border-collapse text-left text-xs">
              <thead className="bg-white/[0.02] text-[10px] uppercase tracking-wider text-slate-500">
                <tr>
                  <Th>#</Th>
                  <Th>Status</Th>
                  <Th>Class</Th>
                  <Th>Cost</Th>
                  <Th>Duration</Th>
                  <Th>Rule</Th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/[0.03]">
                {attempts.map((a) => {
                  const aMeta = runStatusMeta(a.status);
                  return (
                    <tr key={a.attempt_no} className="text-slate-300">
                      <Td className="font-mono text-slate-400">{a.attempt_no}</Td>
                      <Td>
                        <span className={`font-semibold ${TONE_TEXT[aMeta.tone]}`}>
                          {aMeta.label}
                        </span>
                      </Td>
                      <Td className="text-slate-400">
                        {a.error_class ? classLabel(a.error_class) : '—'}
                      </Td>
                      <Td className="font-mono">{formatCost(a.cost_usd)}</Td>
                      <Td className="font-mono">{formatMs(a.wall_clock_ms)}</Td>
                      <Td className="font-mono text-[10px] text-slate-400">
                        {a.applied_rule ?? '—'}
                      </Td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Raw run-event log (P1.13). The standalone `RunEventTimeline` is no
          longer a separate surface (P2.6) — its feed lives here, one shape for
          both transports (local push / remote poll). Run-level, not per-node,
          so it's shown whenever there's a feed to read. */}
      {runEvents && runEvents.length > 0 && (
        <div>
          <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
            Run activity
          </div>
          <div className="max-h-48 space-y-2 overflow-y-auto rounded-xl border border-white/5 bg-[#050608] p-3 font-mono text-[11px]">
            <RunEventFeed events={runEvents} />
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * A sequence node's task list, fetched from `sequence_tasks_list` (P2.5).
 *
 * The point is Decision 13's *landed prefix*: tasks whose commit is already on
 * the feature branch (`landed`) are the work a crash-resume or targeted retry
 * will not re-run. They render solid with a filled check and an emerald rail;
 * pending tasks dim; the live/failed task takes its run-status tone — so the
 * split the engine has always tracked is legible for the first time.
 */
function SequenceTasks({
  featureId,
  nodeId,
  stepExecutionId,
  version,
}: {
  featureId: string;
  nodeId: string;
  stepExecutionId: string | null;
  version: string;
}) {
  const [state, setState] = useState<SequenceState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!stepExecutionId) {
      setState(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<SequenceState>('sequence_tasks_list', {
      featureId,
      nodeId,
      executionId: stepExecutionId,
    })
      .then((s) => {
        if (!cancelled) setState(s);
      })
      .catch((err) => {
        if (!cancelled) setError(formatError(err) || 'Failed to load task list.');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // Refetch as the node advances (a task landing changes the split).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [featureId, nodeId, stepExecutionId, version]);

  const tasks = state?.tasks ?? [];
  const landedCount = tasks.filter((t) => t.landed).length;

  // Nothing to show until the node has planned. Stay silent rather than render
  // an empty box — a sequence node that hasn't reached its plan is the norm.
  if (!stepExecutionId || (!loading && state && !state.planned)) return null;

  return (
    <div>
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
          Task list
        </span>
        {tasks.length > 0 && (
          <span className="font-mono text-[10px] text-emerald-400/80">
            {landedCount}/{tasks.length} landed
          </span>
        )}
      </div>

      {loading && !state ? (
        <div className="flex items-center gap-2 py-6 text-xs text-slate-500">
          <Loader2 className="h-4 w-4 animate-spin text-violet-400" /> Loading task list…
        </div>
      ) : error ? (
        <div className="flex items-start gap-2 rounded-lg border border-rose-500/20 bg-rose-950/20 p-3 text-xs text-rose-300">
          <AlertCircle className="mt-px h-4 w-4 shrink-0 text-rose-400" />
          <span>{error}</span>
        </div>
      ) : tasks.length === 0 ? (
        <EmptyHint>No tasks in this node&apos;s plan.</EmptyHint>
      ) : (
        <ol className="space-y-1.5">
          {groupByCycle(tasks).map((group) => (
            <Fragment key={group.cycle}>
              {/* Only labelled once a rework cycle exists — a single-cycle
                  node is the norm and a "Cycle 0" header on it is noise. */}
              {group.labelled && (
                <li className="flex items-baseline justify-between px-1 pb-0.5 pt-2 first:pt-0">
                  <span className="text-[10px] font-bold uppercase tracking-widest text-violet-400/70">
                    {group.cycle === 0 ? 'Original decomposition' : `Rework ${group.cycle}`}
                  </span>
                  <span className="font-mono text-[10px] text-slate-500">
                    {group.tasks.length} {group.tasks.length === 1 ? 'ticket' : 'tickets'}
                  </span>
                </li>
              )}
              {group.tasks.map((t, i) => (
                <SequenceTaskRow key={`${group.cycle}-${t.id}`} index={i + 1} task={t} />
              ))}
            </Fragment>
          ))}
        </ol>
      )}
    </div>
  );
}

/**
 * Split a flat task list into its decomposition cycles, in order.
 *
 * A step that a downstream verdict sent back has planned more than one list:
 * the original decomposition, then one delta per rework cycle. Both are on the
 * branch, so both are shown — rendering only the list that ran last would
 * present a four-ticket delta as if it were the whole feature.
 *
 * `labelled` is false for the common single-cycle node, where a header would
 * name a distinction that isn't there yet.
 */
function groupByCycle(
  tasks: SequenceState['tasks'],
): { cycle: number; tasks: SequenceState['tasks']; labelled: boolean }[] {
  const groups: { cycle: number; tasks: SequenceState['tasks']; labelled: boolean }[] = [];
  for (const task of tasks) {
    const cycle = task.cycle ?? 0;
    const last = groups[groups.length - 1];
    if (last && last.cycle === cycle) last.tasks.push(task);
    else groups.push({ cycle, tasks: [task], labelled: false });
  }
  const multi = groups.length > 1;
  return groups.map((g) => ({ ...g, labelled: multi }));
}

function SequenceTaskRow({
  index,
  task,
}: {
  index: number;
  task: SequenceState['tasks'][number];
}) {
  const meta = runStatusMeta(task.landed ? 'completed' : task.status);
  const isFailed = task.status === 'failed' || task.status === 'interrupted';
  const isActive = task.status === 'running';

  return (
    <li
      className={`flex items-start gap-3 rounded-lg border-l-2 py-2 pl-3 pr-2 ${
        task.landed
          ? 'border-emerald-500/60 bg-emerald-500/[0.04]'
          : isFailed
            ? 'border-rose-500/50 bg-rose-500/[0.04]'
            : isActive
              ? 'border-cyan-500/50 bg-cyan-500/[0.04]'
              : 'border-white/5 bg-white/[0.01] opacity-70'
      }`}
    >
      {/* Landed/pending glyph */}
      <span className="mt-0.5 shrink-0">
        {task.landed ? (
          <Check className="h-3.5 w-3.5 text-emerald-400" />
        ) : isActive ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-cyan-400" />
        ) : isFailed ? (
          <XCircle className="h-3.5 w-3.5 text-rose-400" />
        ) : (
          <CircleDashed className="h-3.5 w-3.5 text-slate-600" />
        )}
      </span>

      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="shrink-0 font-mono text-[10px] text-slate-500">{index}</span>
          <span className="truncate text-xs text-slate-200" title={task.title || task.id}>
            {task.title || task.id}
          </span>
        </div>
        {task.error_message && isFailed && (
          <div className="mt-1 truncate text-[10px] text-rose-300/70" title={task.error_message}>
            {task.error_message}
          </div>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-2">
        {typeof task.cost_usd === 'number' && task.cost_usd > 0 && (
          <span className="font-mono text-[10px] text-emerald-400/80">
            {formatCost(task.cost_usd)}
          </span>
        )}
        <span
          className={`rounded px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider ${TONE_CHIP[meta.tone]}`}
        >
          {task.landed ? 'Landed' : meta.label}
        </span>
      </div>
    </li>
  );
}

/** Live: the running node's agent-stream buffer (same source as the timeline). */
function LiveTab({ liveStream, isStreaming }: { liveStream?: string; isStreaming: boolean }) {
  const content = liveStream?.trim() || '';
  if (!content) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center text-xs text-slate-500">
        {isStreaming ? (
          <>
            <Cpu className="h-5 w-5 animate-spin text-cyan-400" />
            <span>Waiting for agent output…</span>
          </>
        ) : (
          <span className="font-bold uppercase tracking-wider text-slate-600">
            No live output — this node isn&apos;t running.
          </span>
        )}
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col overflow-hidden px-5 py-4">
      <div className="mb-2 flex shrink-0 items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
        {isStreaming && <Cpu className="h-3 w-3 animate-spin text-cyan-400" />}
        Agent reasoning
      </div>
      {/* Newest at the bottom; `flex-col-reverse` keeps it scrolled to live. */}
      <div className="flex min-h-0 flex-1 flex-col-reverse overflow-y-auto rounded-lg border border-cyan-500/20 bg-[#020304] p-3">
        <pre className="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-cyan-300/80">
          {content}
        </pre>
      </div>
    </div>
  );
}

/** Output: declared artifacts (Monaco) + harness/verifier output. */
function OutputTab({
  step,
  hasOutput,
  artifactPaths,
  selectedArtifact,
  onSelectArtifact,
  onOpenEditorForPath,
  onOpenArtifact,
}: {
  step: StepExecution | null;
  hasOutput: boolean;
  artifactPaths: string[];
  selectedArtifact: string | null;
  onSelectArtifact: (path: string) => void;
  onOpenEditorForPath?: (filePath: string) => void;
  onOpenArtifact?: (artifactPath: string) => void;
}) {
  // Cache-bust the viewer the same way the timeline does: a re-pull can
  // overwrite an artifact at the same path, so key on what changes on a fresh
  // attempt (status/tokens/duration/cost).
  const contentVersion = step
    ? `${step.status}:${step.tokens}:${step.wall_clock_secs}:${step.cost_usd}`
    : undefined;
  const errorOutput = step?.error_message?.trim() || null;
  const isFailed = step?.status === 'failed' || step?.status === 'interrupted';

  if (!hasOutput) {
    return (
      <div className="flex h-full items-center justify-center px-8 text-center text-xs font-bold uppercase tracking-wider text-slate-600">
        No output produced for this node.
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden px-5 py-4">
      {/* Harness / verifier output — the failing-tests / implicated-files surface. */}
      {errorOutput && (
        <div className="mb-4 shrink-0">
          <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
            {isFailed ? 'Verifier / harness output' : 'Message'}
          </div>
          <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-xl border border-rose-500/20 bg-rose-950/10 p-3 font-mono text-[11px] leading-relaxed text-rose-200/90">
            {errorOutput}
          </pre>
        </div>
      )}

      {/* Artifact chooser */}
      {artifactPaths.length > 0 && (
        <div className="mb-3 shrink-0 space-y-2">
          <div className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
            Artifacts
          </div>
          {artifactPaths.map((path) => {
            const cls = classifyArtifact(path);
            // Nothing stays "selected" when the host owns the preview — the
            // modal is the selection.
            const selected = !onOpenArtifact && selectedArtifact === path;
            return (
              <button
                key={path}
                onClick={() => (onOpenArtifact ? onOpenArtifact(path) : onSelectArtifact(path))}
                className={`flex w-full items-center gap-3 rounded border p-2.5 text-left font-mono text-xs transition ${
                  selected
                    ? 'border-violet-500/30 bg-violet-950/20 text-violet-300 shadow-[0_0_15px_rgba(139,92,246,0.1)]'
                    : 'border-white/[0.02] bg-[#050608] text-slate-400 hover:border-white/10 hover:bg-white/[0.02] hover:text-white'
                }`}
              >
                <span className={ARTIFACT_KIND_COLORS[cls.kind]}>
                  <ArtifactIcon kind={cls.kind} />
                </span>
                <span className="flex-1 truncate">{cls.basename}</span>
                <span className="shrink-0 text-[9px] font-bold uppercase text-slate-500">
                  {ARTIFACT_KIND_LABELS[cls.kind]}
                </span>
              </button>
            );
          })}
        </div>
      )}

      {/* Selected artifact body — only when the host hasn't taken the preview
          over via `onOpenArtifact`. */}
      {!onOpenArtifact && selectedArtifact && (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <ArtifactViewer
            artifactPath={selectedArtifact}
            contentVersion={contentVersion}
            onOpenEditorForPath={onOpenEditorForPath}
          />
        </div>
      )}
    </div>
  );
}

/** Actions: retry / replay / stop / decide-gate, with the ancestor guard. */
function ActionsTab({
  node,
  run,
  hasActions,
  attempts,
  blockedBy,
  onRetry,
  onReplay,
  onStop,
  onDecideGate,
}: {
  node: NodeConfigV2;
  run: NodeRunStatus | null;
  hasActions: boolean;
  attempts: StepAttempt[];
  blockedBy: BlockingAncestor | null;
  onRetry?: () => void;
  onReplay?: () => void;
  onStop?: () => void;
  onDecideGate?: () => void;
}) {
  const status = run?.status ?? 'pending';
  const isFailed = status === 'failed' || status === 'interrupted';
  const isRunning = status === 'running' || status === 'verifying';
  const isGateWaiting = node.type === 'gate' && status === 'awaiting_gate';
  const guarded = blockedBy !== null;
  const guardMsg = blockedBy
    ? `Ancestor "${blockedBy.step_id}" is still ${blockedBy.status}. Wait for it to finish.`
    : '';

  // The policy rule the engine applied to this node's most recent failure — the
  // "which rule will apply" hint (P2.4), read straight from the attempt row.
  const lastFailed = [...attempts].reverse().find((a) => a.error_class);

  if (!hasActions) {
    return (
      <div className="flex h-full items-center justify-center px-8 text-center text-xs font-bold uppercase tracking-wider text-slate-600">
        No actions available for this node yet.
      </div>
    );
  }

  return (
    <div className="h-full space-y-3 overflow-y-auto px-5 py-4">
      {onDecideGate && isGateWaiting && (
        <ActionRow
          icon={<ShieldCheck className="h-4 w-4" />}
          tone="amber"
          title="Decide gate"
          desc="Open the full-screen review to approve, redirect, or cancel."
          buttonLabel="Decide"
          onClick={onDecideGate}
        />
      )}

      {onRetry && isFailed && (
        <ActionRow
          icon={<RefreshCw className="h-4 w-4" />}
          tone="ruby"
          title="Retry node"
          desc={
            lastFailed?.applied_rule
              ? `Re-run from scratch. Last failure (${classLabel(lastFailed.error_class!)}) was handled by ${lastFailed.applied_rule}.`
              : 'Re-run this node from scratch with the current harness/model.'
          }
          buttonLabel="Retry"
          onClick={onRetry}
          disabled={guarded}
          disabledReason={guardMsg}
        />
      )}

      {onReplay && (
        <ActionRow
          icon={<RotateCcw className="h-4 w-4" />}
          tone="cyan"
          title="Replay from node"
          desc="Re-execute this node and everything downstream. The affected nodes are ringed on the graph before you confirm."
          buttonLabel="Replay…"
          onClick={onReplay}
        />
      )}

      {onStop && isRunning && (
        <ActionRow
          icon={<XCircle className="h-4 w-4" />}
          tone="ruby"
          title="Stop node"
          desc="Cancel the in-flight execution."
          buttonLabel="Stop"
          onClick={onStop}
        />
      )}

      {guarded && (
        <div className="flex items-start gap-2 rounded-lg border border-amber-500/20 bg-amber-950/10 p-3 text-xs text-amber-300/90">
          <AlertCircle className="mt-px h-4 w-4 shrink-0 text-amber-400" />
          <span>{guardMsg}</span>
        </div>
      )}
    </div>
  );
}

const ACTION_TONE: Record<string, string> = {
  ruby: 'border-rose-500/20 bg-rose-950/10',
  cyan: 'border-cyan-500/20 bg-cyan-950/10',
  amber: 'border-amber-500/20 bg-amber-950/10',
};
const ACTION_BTN: Record<string, string> = {
  ruby: 'bg-rose-600 hover:bg-rose-500 text-white',
  cyan: 'bg-cyan-600 hover:bg-cyan-500 text-white',
  amber: 'bg-amber-500 hover:bg-amber-600 text-black',
};

function ActionRow({
  icon,
  tone,
  title,
  desc,
  buttonLabel,
  onClick,
  disabled,
  disabledReason,
}: {
  icon: React.ReactNode;
  tone: 'ruby' | 'cyan' | 'amber';
  title: string;
  desc: string;
  buttonLabel: string;
  onClick: () => void;
  disabled?: boolean;
  disabledReason?: string;
}) {
  return (
    <div className={`flex items-center gap-3 rounded-xl border p-3.5 ${ACTION_TONE[tone]}`}>
      <div className={`shrink-0 ${TONE_TEXT[tone as keyof typeof TONE_TEXT] ?? 'text-slate-400'}`}>
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-xs font-bold uppercase tracking-wider text-slate-200">{title}</div>
        <div className="mt-0.5 text-[11px] leading-relaxed text-slate-400">{desc}</div>
      </div>
      <button
        onClick={onClick}
        disabled={disabled}
        title={disabled ? disabledReason : undefined}
        className={`shrink-0 rounded-lg px-3 py-1.5 text-xs font-bold transition disabled:cursor-not-allowed disabled:bg-slate-700/40 disabled:text-slate-500 ${ACTION_BTN[tone]}`}
      >
        {buttonLabel}
      </button>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2">
      <div className="text-[9px] font-bold uppercase tracking-widest text-slate-500">{label}</div>
      <div className="mt-0.5 font-mono text-sm text-slate-200">{value}</div>
    </div>
  );
}

function EmptyHint({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-white/5 bg-white/[0.01] px-3 py-4 text-center text-xs text-slate-500">
      {children}
    </div>
  );
}

function Th({ children }: { children: React.ReactNode }) {
  return <th className="px-3 py-2 font-semibold">{children}</th>;
}

function Td({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return <td className={`px-3 py-2 ${className}`}>{children}</td>;
}
