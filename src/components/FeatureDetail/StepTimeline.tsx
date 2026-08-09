import { useMemo, useRef, type MutableRefObject } from 'react';
import { RefreshCw } from 'lucide-react';
import type { HarnessBaseline, RemoteRunMirror, StepExecution } from '../../types';
import { findActivePredecessor } from '../../lib/features';
import { StepCard } from './StepCard';
import type { AgentStreamStore } from './useAgentStream';
import { useStreamText, useStreamTruncated } from './useAgentStream';
import type { HarnessOverrides } from './useHarnessOverrides';
import type { ReplayTarget } from './useRerunActions';

interface StepTimelineProps {
  steps: StepExecution[];
  remoteRun: RemoteRunMirror | null;
  remoteMachineName: string | null;
  hasBootstrapPhases: boolean;
  gateStepExecutionId: string | null | undefined;
  stepCardRefs: MutableRefObject<Record<string, HTMLDivElement | null>>;
  harnessBaseline: HarnessBaseline | null;
  overrides: HarnessOverrides;
  selectedArtifactPath: string | null;
  /** Execution id the inspector *resolved* to, not the raw selection: a node id
   *  picked on the canvas matches several rows once a step has been retried,
   *  and only the attempt on screen may read as selected. */
  selectedStepId: string | null;
  activeStreamId: string | null;
  streamStore: AgentStreamStore;
  onSelect: (stepExecutionId: string) => void;
  onToggleStream: (stepExecutionId: string) => void;
  onOpenArtifact: (path: string, stepTitle: string) => void;
  onStartReplay: (target: ReplayTarget) => void;
  onRetry: (stepExecutionId: string) => void;
  onStop: () => void;
  onDecideGate: (stepExecutionId: string) => void;
}

/**
 * The run as a vertical list — best for skimming a long linear run, and the
 * only surface a feature with no workflow graph has.
 *
 * Clicking a row selects the step the shared inspector reads
 * (UI_REDESIGN_PLAN §3.1). The `steps.map` gets its own element so the
 * pre-hydration banner above it is not an item in the list.
 *
 * Selection is `aria-current`, deliberately not the `listbox`/`option` pair it
 * was first written as: a card carries its own replay, gate, stream and stop
 * buttons, and an `option` may not own interactive children — so no placement
 * of that role is valid here, including on the card root.
 */
export function StepTimeline({
  steps,
  remoteRun,
  remoteMachineName,
  hasBootstrapPhases,
  gateStepExecutionId,
  stepCardRefs,
  harnessBaseline,
  overrides,
  selectedArtifactPath,
  selectedStepId,
  activeStreamId,
  streamStore,
  onSelect,
  onToggleStream,
  onOpenArtifact,
  onStartReplay,
  onRetry,
  onStop,
  onDecideGate,
}: StepTimelineProps) {
  // Only the open card renders a stream, so this is the one subscription the
  // timeline needs — and with no card open it subscribes to nothing at all.
  const openStream = useStreamText(streamStore, activeStreamId);
  const openStreamTruncated = useStreamTruncated(streamStore, activeStreamId);

  // Aligned with `steps` by index. Recomputed only when the list itself
  // changes, rather than once per card per stream frame.
  const predecessors = useMemo(
    () => steps.map((step) => findActivePredecessor(steps, step)),
    [steps],
  );

  /** One ref callback per step, cached: a fresh closure per render would change
   *  a memoized card's props on every frame. */
  const cardRefCache = useRef(new Map<string, (el: HTMLDivElement | null) => void>());
  const cardRefFor = (stepExecutionId: string) => {
    const cached = cardRefCache.current.get(stepExecutionId);
    if (cached) return cached;
    const collect = (el: HTMLDivElement | null) => {
      stepCardRefs.current[stepExecutionId] = el;
    };
    cardRefCache.current.set(stepExecutionId, collect);
    return collect;
  };

  return (
    <div className="relative shrink-0 border-l border-white/5 ml-4 pl-8 space-y-6">
      {remoteRun && steps.length === 0 && !hasBootstrapPhases && (
        /* Eager shadow, pre-hydration: the run was submitted a
           moment ago and the runner hasn't bootstrapped a
           feature yet, so there are no shadow steps to mirror.
           `useRemoteRun`'s poll fills this in once the runner
           reports them. Suppressed once the richer bootstrap
           stepper has phases to show. */
        <div className="glass-panel p-6 border border-cyan-500/20">
          <div className="flex items-center gap-3">
            <RefreshCw className="w-5 h-5 text-cyan-400 animate-spin shrink-0" />
            <div>
              <h3 className="text-sm font-semibold text-white">
                Submitted to {remoteMachineName ?? remoteRun.machine_id}
              </h3>
              <p className="text-xs text-slate-400 mt-1">
                The runner is cloning the repository and bootstrapping the workflow.
                Steps appear here automatically — you can close Demeteo; the run continues.
              </p>
            </div>
          </div>
        </div>
      )}
      <ul aria-label="Run steps" className="space-y-6">
        {steps.map((step, idx) => (
          <StepCard
            key={step.id}
            step={step}
            index={idx}
            downstreamCount={steps.length - idx - 1}
            activePredecessor={predecessors[idx]}
            isActiveGate={gateStepExecutionId === step.id}
            isSelected={selectedStepId === step.id}
            cardRef={cardRefFor(step.id)}
            harnessBaseline={harnessBaseline}
            overrides={overrides}
            selectedArtifactPath={selectedArtifactPath}
            isStreamOpen={activeStreamId === step.id}
            stream={activeStreamId === step.id ? openStream : ''}
            streamTruncated={activeStreamId === step.id ? openStreamTruncated : false}
            onSelect={onSelect}
            onToggleStream={onToggleStream}
            onOpenArtifact={onOpenArtifact}
            onStartReplay={onStartReplay}
            onRetry={onRetry}
            onStop={onStop}
            onDecideGate={onDecideGate}
          />
        ))}
      </ul>
    </div>
  );
}
