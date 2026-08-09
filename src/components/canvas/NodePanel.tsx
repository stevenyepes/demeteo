/**
 * `NodePanel` — the node drill-down side panel (tasks P2.3 + P2.4, PRD §6.2).
 *
 * It answers J2 ("which node, which attempt, what did it cost, why did it fail")
 * for the selected node from data Phase 1 persists, across four tabs —
 * Overview, Live, Output, Actions, each in `nodePanel/`.
 *
 * It belongs to no one surface. `FeatureDetail/StepInspector.tsx` docks one of
 * these beside *either* run surface and feeds it whichever step is selected, so
 * a timeline row and a graph node reach the same four tabs. A `node` for a run
 * with no graph definition is synthesized from the step
 * (`FeatureDetail/stepIdentity.ts`), which is why nothing here may assume the
 * node came from a stored definition — the canvas's own types are the shape
 * this panel reads, not evidence of where the data came from.
 *
 * What stays here is what more than one tab needs: the attempt history (the
 * Overview table and the Actions retry hint read the same rows, so it is
 * fetched once), the artifact selection, and the tab. Everything else is a
 * tab's own business. The shell itself is the generic `Inspector` primitive,
 * whose strip is `TabBar` at its dense size.
 */
import { useEffect, useMemo, useState } from 'react';

import { Inspector } from '../ui/Inspector';
import type { TabDef } from '../ui/TabBar';
import { formatError } from '../../lib/errors';
import { listStepAttempts } from '../../lib/features';
import { runStatusMeta, TONE_CHIP, TONE_TEXT } from '../../lib/runStatus';
import type { AgentStreamStore } from '../FeatureDetail/useAgentStream';
import type { RunEvent, StepAttempt, StepExecution } from '../../types';
import { nodeTypeMeta, type NodeConfigV2, type NodeRunStatus } from './types';
import { ActionsTab, type BlockingAncestor } from './nodePanel/ActionsTab';
import { LiveTab } from './nodePanel/LiveTab';
import { OutputTab } from './nodePanel/OutputTab';
import { OverviewTab } from './nodePanel/OverviewTab';
import { classLabel } from './nodePanel/format';

export type { BlockingAncestor };

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
  /** Where the Live tab reads the backing execution's `agent_stream` buffer.
   *  The store rather than the text: the panel hands it straight to `LiveTab`,
   *  which subscribes, so a stream wakes nothing while another tab is showing.
   *  Absent = no run behind this panel and a Live tab that says so. */
  streamStore?: AgentStreamStore;
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
  /** Sizing and placement belong to whatever docks the panel — the run view
   *  seats it in a `SplitPane` the user drags, so the panel states no width of
   *  its own. */
  className?: string;
}

type Tab = 'overview' | 'live' | 'output' | 'actions';

const TABS: readonly TabDef<Tab>[] = [
  { value: 'overview', label: 'Overview' },
  { value: 'live', label: 'Live' },
  { value: 'output', label: 'Output' },
  { value: 'actions', label: 'Actions' },
];

export function NodePanel({
  featureId,
  node,
  run,
  step,
  onClose,
  onOpenEditorForPath,
  onOpenArtifact,
  runEvents,
  streamStore,
  isStreaming,
  blockedBy,
  onRetry,
  onReplay,
  onStop,
  onDecideGate,
  className = '',
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
    listStepAttempts(stepExecutionId)
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
    <Inspector
      className={className}
      title={node.title}
      icon={<TypeIcon className={`h-4 w-4 shrink-0 ${TONE_TEXT[meta.tone]}`} />}
      meta={
        <>
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
        </>
      }
      onDismiss={onClose}
      ariaLabel="Node detail"
      tabs={TABS}
      activeTab={tab}
      onTabChange={setTab}
    >
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
      {tab === 'live' && (
        <LiveTab
          streamStore={streamStore}
          stepExecutionId={stepExecutionId}
          isStreaming={!!isStreaming}
        />
      )}
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
    </Inspector>
  );
}
