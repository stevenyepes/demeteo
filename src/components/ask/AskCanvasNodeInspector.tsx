/**
 * The Ask Canvas node detail pane (`docs/ask-canvas/probe/CanvasFocus.html`).
 * No tabs — unlike `NodePanel.tsx`'s 4-tab use of `Inspector`, this is a
 * single scrolling body, so it builds directly on `INSPECTOR_SURFACE` rather
 * than `Inspector` itself (see that component's doc comment).
 */
import { useEffect, useState } from 'react';
import { ExternalLink, Workflow as PipelineIcon, X } from 'lucide-react';

import { useNavigation } from '../../context';
import { useErrorBus } from '../../lib/errorBus';
import { resolveNode } from '../../lib/ask';
import { fetchActiveFeatures, listStepsForRun } from '../../lib/features';
import type { CanvasNeighbor } from '../../lib/askCanvasEdges';
import type { CanvasNode, Feature, NodeResolution, StepExecution } from '../../types';
import { INSPECTOR_SURFACE } from '../ui/Inspector';
import { ROLE_CHIP, ROLE_ICON, ROLE_LABEL } from './AskCanvasNode';

export interface AskCanvasNodeInspectorProps {
  node: CanvasNode;
  /** Already resolved by the caller via `descriptionForNode(...)`, role-label
   *  fallback included — this component never re-derives it. */
  description: string;
  incoming: CanvasNeighbor[];
  outgoing: CanvasNeighbor[];
  threadId: string;
  messageId: string;
  projectId: string;
  onDismiss: () => void;
}

/** `absent` is a node that never named a file — `resolve` refuses those by
 *  contract, so this component must not ask. See `NodePathState` in
 *  `AskCanvasNode.tsx` for the same three-way split on the card. */
type Resolution =
  | { status: 'absent' }
  | { status: 'pending' }
  | { status: 'ready'; result: NodeResolution };

interface PipelineMatch {
  featureId: string;
  featureTitle: string;
  stepId: string;
}

/** §0's exact-match rule: a node maps to a pipeline destination only when its
 *  path is exactly a Step's `artifact_path` or a member of `artifact_paths`. */
function findPipelineMatch(
  features: readonly Feature[],
  stepsByFeature: readonly StepExecution[][],
  path: string,
): PipelineMatch | null {
  for (let i = 0; i < features.length; i++) {
    const feature = features[i];
    for (const step of stepsByFeature[i] ?? []) {
      if (step.artifact_path === path || step.artifact_paths.includes(path)) {
        return { featureId: feature.id, featureTitle: feature.title, stepId: step.step_id };
      }
    }
  }
  return null;
}

export function AskCanvasNodeInspector({
  node,
  description,
  incoming,
  outgoing,
  threadId,
  messageId,
  projectId,
  onDismiss,
}: AskCanvasNodeInspectorProps) {
  const { navigate } = useNavigation();
  const { reportError } = useErrorBus();

  const [resolution, setResolution] = useState<Resolution>({ status: 'pending' });
  const [pipelineMatch, setPipelineMatch] = useState<PipelineMatch | null>(null);

  useEffect(() => {
    let cancelled = false;
    if (node.path === null) {
      setResolution({ status: 'absent' });
      return;
    }
    setResolution({ status: 'pending' });
    resolveNode({ threadId, messageId, nodeId: node.id })
      .then((result) => {
        if (!cancelled) setResolution({ status: 'ready', result });
      })
      .catch((err) => {
        if (!cancelled) reportError(err);
      });
    return () => {
      cancelled = true;
    };
  }, [threadId, messageId, node.id, node.path, reportError]);

  useEffect(() => {
    let cancelled = false;
    setPipelineMatch(null);
    const path = node.path;
    if (path === null) return;
    fetchActiveFeatures(projectId)
      .then(async (features) => {
        const stepsByFeature = await Promise.all(features.map((feature) => listStepsForRun(feature.id)));
        if (!cancelled) setPipelineMatch(findPipelineMatch(features, stepsByFeature, path));
      })
      .catch((err) => {
        if (!cancelled) reportError(err);
      });
    return () => {
      cancelled = true;
    };
  }, [projectId, node.path, reportError]);

  const handleOpenInEditor = () => {
    if (resolution.status !== 'ready' || resolution.result.kind !== 'editor') return;
    const result = resolution.result;
    navigate({
      kind: 'editor',
      editorContext: {
        machineId: result.machine_id,
        worktreePath: result.worktree_path,
        branch: result.branch,
        defaultBranch: result.default_branch,
        initialFile: result.path,
      },
    });
  };

  const handleShowInPipeline = () => {
    if (!pipelineMatch) return;
    navigate({
      kind: 'detail',
      featureId: pipelineMatch.featureId,
      featureTitle: pipelineMatch.featureTitle,
      selectedStepId: pipelineMatch.stepId,
    });
  };

  const Icon = ROLE_ICON[node.role];
  const sectionLabel = 'text-[10px] font-mono uppercase tracking-wider text-slate-500 mb-2';

  return (
    <div data-testid="ask-canvas-node-inspector" className={INSPECTOR_SURFACE}>
      <div className="flex shrink-0 items-start justify-between gap-3 border-b border-white/5 px-5 py-4">
        <div className="flex min-w-0 items-start gap-3">
          <div
            className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border ${ROLE_CHIP[node.role]}`}
          >
            <Icon className="h-4 w-4" aria-hidden />
          </div>
          <div className="min-w-0">
            <h3 className="truncate font-heading text-sm font-semibold text-white">{node.title}</h3>
            <span
              className={`mt-1.5 inline-flex items-center rounded-full border px-2 py-0.5 font-mono text-[10px] uppercase tracking-wide ${ROLE_CHIP[node.role]}`}
            >
              {ROLE_LABEL[node.role]}
            </span>
          </div>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          aria-label="Close panel"
          className="shrink-0 rounded-lg bg-white/5 p-1.5 text-slate-400 transition hover:bg-white/10 hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4">
        <section>
          <div className={sectionLabel}>What happens here</div>
          <p className="text-[12.5px] leading-relaxed text-slate-300">{description}</p>
        </section>

        {node.path !== null && (
        <section>
          <div className={sectionLabel}>In the code</div>
          {resolution.status === 'ready' && resolution.result.kind === 'moved' ? (
            <p className="text-[12.5px] leading-relaxed text-slate-300">
              <span className="font-mono text-slate-400">{node.path}</span> moved since{' '}
              {resolution.result.checked_commit_sha.slice(0, 8)}
            </p>
          ) : (
            <div className="truncate rounded-lg border border-white/5 bg-white/5 px-2.5 py-2 font-mono text-[11px] text-slate-300">
              {resolution.status === 'ready' && resolution.result.kind === 'editor'
                ? resolution.result.path
                : node.path}
            </div>
          )}
        </section>
        )}

        <section>
          <div className={sectionLabel}>Edges</div>
          <div className="space-y-1">
            {incoming.map((neighbor) => (
              <div key={`in-${neighbor.nodeId}`} className="text-[12px] text-slate-400">
                in — <span className="font-mono text-[11px] text-slate-300">{neighbor.title}</span>{' '}
                <span className="text-slate-600">({neighbor.kind})</span>
              </div>
            ))}
            {outgoing.map((neighbor) => (
              <div key={`out-${neighbor.nodeId}`} className="text-[12px] text-slate-400">
                out — <span className="font-mono text-[11px] text-slate-300">{neighbor.title}</span>{' '}
                <span className="text-slate-600">({neighbor.kind})</span>
              </div>
            ))}
          </div>
        </section>
      </div>

      <div className="flex shrink-0 flex-col gap-2 border-t border-white/5 px-5 py-4">
        {resolution.status === 'ready' && resolution.result.kind === 'editor' && (
          <button
            type="button"
            onClick={handleOpenInEditor}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-gradient-to-br from-violet-600 to-violet-700 px-3.5 py-2 text-xs font-semibold text-white"
          >
            <ExternalLink className="h-3.5 w-3.5" aria-hidden />
            Open in editor
          </button>
        )}
        {pipelineMatch && (
          <button
            type="button"
            onClick={handleShowInPipeline}
            className="flex w-full items-center justify-center gap-2 rounded-lg border border-white/10 px-3.5 py-2 text-xs font-medium text-slate-300"
          >
            <PipelineIcon className="h-3.5 w-3.5" aria-hidden />
            Show in the pipeline
          </button>
        )}
      </div>
    </div>
  );
}

export default AskCanvasNodeInspector;
