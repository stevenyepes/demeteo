/**
 * `NodePanel` — the node drill-down side panel (task P2.3, PRD §6.2).
 *
 * Clicking a node on the run-mode `WorkflowCanvas` opens this panel beside the
 * graph (the same split-panel shape `ArtifactViewer` uses in the timeline). It
 * answers J2 ("which node, which attempt, what did it cost, why did it fail")
 * for the selected node from data Phase-1 now persists:
 *
 *  - **Overview** — status, failure class, the per-attempt table from
 *    `step_attempts` (class · cost · duration · applied rule), so a retry loop
 *    is legible instead of collapsed onto one row.
 *  - **Output** — the node's declared artifacts (Monaco via `ArtifactViewer`)
 *    plus the harness/verifier output (`error_message`, which carries failing
 *    tests / implicated files today).
 *
 * Later tabs (Live transcript, Actions) land in P2.4; this task ships the two
 * read-only tabs that make the failure→root-cause path ≤3 clicks.
 */
import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, Loader2, X } from 'lucide-react';

import { ArtifactViewer } from '../ArtifactViewer';
import {
  ArtifactIcon,
  ARTIFACT_KIND_COLORS,
  ARTIFACT_KIND_LABELS,
  classifyArtifact,
} from '../../lib/artifacts';
import { formatError } from '../../lib/errors';
import { formatDuration } from '../../lib/utils';
import {
  runStatusMeta,
  TONE_CHIP,
  TONE_TEXT,
} from '../../lib/runStatus';
import type { StepAttempt, StepExecution } from '../../types';
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
}

type Tab = 'overview' | 'output';

export function NodePanel({
  featureId: _featureId,
  node,
  run,
  step,
  onClose,
  onOpenEditorForPath,
}: NodePanelProps) {
  const [tab, setTab] = useState<Tab>('overview');
  const meta = nodeTypeMeta(node.type);
  const TypeIcon = meta.icon;

  const status = run?.status ?? 'pending';
  const statusMeta = runStatusMeta(status);
  const errorClass = run?.errorClass ?? null;

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

  return (
    <div className="flex h-full w-[62%] min-w-0 flex-col border-l border-white/5 bg-[#0d0f14]/80 backdrop-blur-xl">
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
        <TabButton active={tab === 'output'} onClick={() => setTab('output')}>
          Output
        </TabButton>
      </div>

      {/* Body */}
      <div className="min-h-0 flex-1 overflow-hidden">
        {tab === 'overview' ? (
          <OverviewTab run={run} stepExecutionId={run?.stepExecutionId ?? null} />
        ) : (
          <OutputTab
            step={step}
            hasOutput={hasOutput}
            artifactPaths={artifactPaths}
            selectedArtifact={selectedArtifact}
            onSelectArtifact={setSelectedArtifact}
            onOpenEditorForPath={onOpenEditorForPath}
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

/** Overview: node totals + the per-attempt history table (`step_attempts`). */
function OverviewTab({
  run,
  stepExecutionId,
}: {
  run: NodeRunStatus | null;
  stepExecutionId: string | null;
}) {
  const [attempts, setAttempts] = useState<StepAttempt[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Refetch when the backing execution changes or advances (status/cost move as
  // a new attempt closes), so the table tracks the live run without polling.
  const version = `${run?.status}:${run?.costUsd}:${run?.wallClockSecs}`;
  useEffect(() => {
    if (!stepExecutionId) {
      setAttempts([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<StepAttempt[]>('step_attempts_list', { executionId: stepExecutionId })
      .then((rows) => {
        if (!cancelled) setAttempts(rows);
      })
      .catch((err) => {
        if (!cancelled) setError(formatError(err) || 'Failed to load attempts.');
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stepExecutionId, version]);

  return (
    <div className="h-full space-y-5 overflow-y-auto px-5 py-4">
      {/* Totals */}
      <div className="grid grid-cols-3 gap-3">
        <Stat label="Attempts" value={run ? String(Math.max(attempts.length, 1)) : '—'} />
        <Stat label="Total cost" value={formatCost(run?.costUsd)} />
        <Stat label="Duration" value={run?.wallClockSecs != null ? formatDuration(run.wallClockSecs) : '—'} />
      </div>

      {/* Attempt table */}
      <div>
        <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
          Attempt history
        </div>
        {!stepExecutionId ? (
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
}: {
  step: StepExecution | null;
  hasOutput: boolean;
  artifactPaths: string[];
  selectedArtifact: string | null;
  onSelectArtifact: (path: string) => void;
  onOpenEditorForPath?: (filePath: string) => void;
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
            const selected = selectedArtifact === path;
            return (
              <button
                key={path}
                onClick={() => onSelectArtifact(path)}
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

      {/* Selected artifact body */}
      {selectedArtifact && (
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
