import type { MutableRefObject } from 'react';
import { RefreshCw } from 'lucide-react';
import type { HarnessBaseline, RemoteRunMirror, StepExecution } from '../../types';
import { StepCard } from './StepCard';
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
  activeStreamId: string | null;
  streamContent: Record<string, string>;
  onToggleStream: (stepExecutionId: string) => void;
  onOpenArtifact: (path: string, stepTitle: string) => void;
  onStartReplay: (target: ReplayTarget) => void;
  onRetry: (stepExecutionId: string) => void;
  onStop: () => void;
  onDecideGate: (stepExecutionId: string) => void;
}

/** The run as a vertical list — the default view, best for skimming a long linear run. */
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
  activeStreamId,
  streamContent,
  onToggleStream,
  onOpenArtifact,
  onStartReplay,
  onRetry,
  onStop,
  onDecideGate,
}: StepTimelineProps) {
  return (
    <div className="relative shrink-0 border-l border-white/5 ml-4 pl-8 space-y-6">
      {remoteRun && steps.length === 0 && !hasBootstrapPhases && (
        /* Eager shadow, pre-hydration: the run was submitted a
           moment ago and the runner hasn't bootstrapped a
           feature yet, so there are no shadow steps to mirror.
           The 3s remote_refresh_run poll above fills this in as
           soon as the runner reports them. Suppressed once the
           richer bootstrap stepper has phases to show. */
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
      {steps.map((step, idx) => (
        <StepCard
          key={step.id}
          step={step}
          index={idx}
          steps={steps}
          isActiveGate={gateStepExecutionId === step.id}
          cardRef={(el) => { stepCardRefs.current[step.id] = el; }}
          harnessBaseline={harnessBaseline}
          overrides={overrides}
          selectedArtifactPath={selectedArtifactPath}
          activeStreamId={activeStreamId}
          streamContent={streamContent}
          onToggleStream={onToggleStream}
          onOpenArtifact={onOpenArtifact}
          onStartReplay={onStartReplay}
          onRetry={onRetry}
          onStop={onStop}
          onDecideGate={onDecideGate}
        />
      ))}
    </div>
  );
}
