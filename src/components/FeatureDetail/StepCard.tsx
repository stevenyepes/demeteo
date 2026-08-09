import { memo, useCallback } from 'react';
import {
  ArrowRight, CheckCircle, Cpu, Hourglass, RefreshCw, RotateCcw, ShieldAlert, XCircle,
} from 'lucide-react';
import type { HarnessBaseline, StepExecution } from '../../types';
import { formatCost, formatDuration, formatTokens } from '../../lib/utils';
import { isBaselineEnvironmentFailure, parseEnvironmentFailure } from '../../lib/harnessVerdict';
import { EnvironmentNotReadyPanel } from '../EnvironmentNotReadyPanel';
import { StepArtifactList } from './StepArtifactList';
import { StepRerunPanel } from './StepRerunPanel';
import type { HarnessOverrides } from './useHarnessOverrides';
import type { ReplayTarget } from './useRerunActions';

const humanizeStepId = (id: string) => {
  return id
    .replace(/^s-/, '')
    .split('-')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
};

interface StepCardProps {
  step: StepExecution;
  index: number;
  /** Steps a replay from here would re-run after it. */
  downstreamCount: number;
  activePredecessor: StepExecution | null;
  isActiveGate: boolean;
  cardRef: (el: HTMLDivElement | null) => void;
  harnessBaseline: HarnessBaseline | null;
  overrides: HarnessOverrides;
  selectedArtifactPath: string | null;
  isStreamOpen: boolean;
  /** This step's own buffered output — never the whole run's. */
  stream: string;
  streamTruncated: boolean;
  onToggleStream: (stepExecutionId: string) => void;
  onOpenArtifact: (path: string, stepTitle: string) => void;
  onStartReplay: (target: ReplayTarget) => void;
  onRetry: (stepExecutionId: string) => void;
  onStop: () => void;
  onDecideGate: (stepExecutionId: string) => void;
}

function StepCardInner({
  step,
  index,
  downstreamCount,
  activePredecessor,
  isActiveGate,
  cardRef,
  harnessBaseline,
  overrides,
  selectedArtifactPath,
  isStreamOpen,
  stream,
  streamTruncated,
  onToggleStream,
  onOpenArtifact,
  onStartReplay,
  onRetry,
  onStop,
  onDecideGate,
}: StepCardProps) {
  let icon = <Hourglass className="w-4 h-4 text-slate-500 animate-pulse" />;
  let statusBg = 'border-white/5 bg-white/[0.01]';

  if (step.status === 'completed') {
    icon = <CheckCircle className="w-4 h-4 text-emerald-400" />;
    statusBg = 'border-emerald-500/20 bg-emerald-950/5';
  } else if (step.status === 'failed') {
    icon = <XCircle className="w-4 h-4 text-rose-400" />;
    statusBg = 'border-rose-500/20 bg-rose-950/5';
  } else if (step.status === 'running') {
    icon = <Cpu className="w-4 h-4 text-cyan-400 animate-spin" />;
    statusBg = 'border-cyan-500/30 bg-cyan-950/10 shadow-[0_0_15px_rgba(6,182,212,0.05)]';
  } else if (step.status === 'verifying') {
    icon = <RefreshCw className="w-4 h-4 text-violet-400 animate-spin" />;
    statusBg = 'border-violet-500/30 bg-violet-950/10 shadow-[0_0_15px_rgba(139,92,246,0.05)]';
  } else if (step.status === 'awaiting_gate') {
    icon = <ShieldAlert className="w-4 h-4 text-amber-400 animate-bounce" />;
    statusBg = 'border-amber-500/40 bg-amber-950/10 shadow-[0_0_15px_rgba(245,158,11,0.08)]';
  }

  // `null` for every failure that is not the engine's terminal
  // environment message — a verdict, an agent failure, an empty
  // error. Guessing here would dress a real defect up as somebody
  // else's problem.
  const stepEnvironment = parseEnvironmentFailure(step.error_message);
  const isBlockedByPredecessor = (step.status === 'failed' || step.status === 'interrupted') && activePredecessor !== null;

  const selectArtifact = useCallback(
    (path: string) => onOpenArtifact(path, step.step_id),
    [onOpenArtifact, step.step_id],
  );

  return (
    <div className="relative group">
      {/* Connector node circle */}
      <span className="absolute -left-[41px] top-1.5 flex items-center justify-center w-6 h-6 rounded-full bg-[#08090c] border border-white/10">
        <span className="text-[10px] text-slate-400 font-bold">{index + 1}</span>
      </span>

      <div
        ref={cardRef}
        data-step-id={step.id}
        className={`p-5 rounded-xl border transition-all duration-300 ${statusBg} ${isActiveGate ? 'ring-2 ring-amber-500/40' : ''}`}
      >
        {/* Title and metrics are one row while they fit and two
            when they don't — a half-width window squeezes the
            metric group until the last value renders outside the
            card. (It used to be the artifact split column that
            did this; the window alone is reason enough.) */}
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          {/* Wraps rather than truncates: the hover-only Replay
              button still reserves its width, and with a truncate
              here it spent that width shortening the step name to
              "Resea…" in a card that had room for it. */}
          <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
            <span className="shrink-0">{icon}</span>
            <span className="font-semibold text-white tracking-wide text-sm break-words">{humanizeStepId(step.step_id)}</span>
            <span className="text-[9px] px-2 py-0.5 rounded bg-white/5 text-slate-400 font-mono shrink-0">
              {step.step_kind}
            </span>
            {(step.iteration_count ?? 0) > 0 && (
              <span
                className="flex items-center gap-1 text-[9px] px-2 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 font-mono"
                title={`This step has been retried ${step.iteration_count} time${step.iteration_count !== 1 ? 's' : ''}`}
              >
                <RefreshCw className="w-2.5 h-2.5" />
                {step.iteration_count}x
              </span>
            )}
            <button
              onClick={() => onStartReplay({
                id: step.id,
                name: humanizeStepId(step.step_id),
                downstreamCount,
              })}
              className="opacity-0 group-hover:opacity-100 transition-opacity duration-200 flex items-center gap-1 px-2 py-1 rounded text-[10px] text-cyan-400/60 hover:text-cyan-300 hover:bg-cyan-500/10 font-bold uppercase tracking-wider"
              title="Replay from this step"
            >
              <RotateCcw className="w-3 h-3" /> Replay
            </button>
          </div>

          <div className="flex shrink-0 flex-wrap items-center justify-end gap-x-4 gap-y-1 text-xs font-mono">
            {typeof step.cost_usd === 'number' && step.cost_usd > 0 && (
              <span
                className="text-emerald-400 whitespace-nowrap"
                title={`${step.cost_usd.toFixed(4)} USD`}
              >
                {formatCost(step.cost_usd)}
              </span>
            )}
            {typeof step.cache_read_input_tokens === 'number' && step.cache_read_input_tokens > 0 && (
              <span
                className="text-violet-400 whitespace-nowrap"
                title={`${step.cache_read_input_tokens.toLocaleString()} cache-read tokens (live from last turn)`}
              >
                {formatTokens(step.cache_read_input_tokens)}p cache
              </span>
            )}
            {typeof step.tokens === 'number' && <span className="text-cyan-400 whitespace-nowrap">{formatTokens(step.tokens)}</span>}
            {typeof step.wall_clock_secs === 'number' && <span className="text-slate-400 whitespace-nowrap">{formatDuration(step.wall_clock_secs)}</span>}
          </div>
        </div>

        {step.status === 'awaiting_gate' && (
          <div className="mt-4 p-4 rounded bg-amber-500/5 border border-amber-500/20 flex flex-wrap justify-between items-center gap-3 animate-pulse">
            <div className="min-w-0 flex-1 text-xs text-amber-400 font-semibold uppercase tracking-wide">
              Pipeline paused. Awaiting manual review.
            </div>
            <button
              onClick={() => onDecideGate(step.id)}
              className="flex shrink-0 items-center gap-1.5 whitespace-nowrap px-3 py-1.5 bg-amber-500 hover:bg-amber-600 rounded text-xs font-bold text-black transition shadow-[0_0_10px_rgba(245,158,11,0.4)]"
            >
              Decide Gate <ArrowRight className="w-3 h-3" />
            </button>
          </div>
        )}

        {/* Two different things end a step, and they are not the
            same claim. An environment failure is not the feature's
            defect and carries an action, so it gets the
            remediation-first panel; everything else is a verdict
            the feature answers for and keeps the ruby block. */}
        {(step.status === 'failed' || step.status === 'interrupted') && stepEnvironment && (
          <EnvironmentNotReadyPanel
            failure={stepEnvironment}
            atBase={isBaselineEnvironmentFailure(stepEnvironment, harnessBaseline)}
          />
        )}

        {(step.status === 'failed' || step.status === 'interrupted') && !stepEnvironment && step.error_message && (
          <div className="mt-3 p-3 rounded bg-rose-500/5 border border-rose-500/20 text-xs text-rose-400 font-mono">
            {/* The backend composes this message with newlines and an indented
                reproduce line; render it verbatim instead of collapsing it. */}
            <div className="whitespace-pre-wrap">{step.error_message}</div>
          </div>
        )}

        {(step.status === 'failed' || step.status === 'interrupted') && (
          <StepRerunPanel
            step={step}
            isEnvironmentFailure={stepEnvironment !== null}
            activePredecessor={activePredecessor}
            isBlockedByPredecessor={isBlockedByPredecessor}
            overrides={overrides}
            onRetry={() => onRetry(step.id)}
          />
        )}

        <StepArtifactList
          step={step}
          selectedArtifactPath={selectedArtifactPath}
          onSelect={selectArtifact}
        />

        {(step.status === 'running' || step.status === 'verifying') && (
          <div className="mt-3 flex gap-2">
            <button
              onClick={() => onToggleStream(step.id)}
              className="flex-1 text-left text-xs font-mono p-3 rounded border flex items-center justify-between transition duration-300 bg-[#050608] border-white/[0.02] text-cyan-400 hover:border-cyan-500/30 hover:bg-cyan-950/20 cursor-pointer"
            >
              <span className="truncate flex items-center gap-2">
                <Cpu className="w-3 h-3 animate-spin" />
                View Agent Reasoning
              </span>
              <span className="text-[9px] uppercase font-bold text-cyan-500 shrink-0">
                {isStreamOpen ? 'Hide Stream' : 'Live Stream'}
              </span>
            </button>

            <button
              onClick={onStop}
              className="px-4 py-2.5 bg-rose-600/20 hover:bg-rose-600 border border-rose-500/30 text-rose-400 hover:text-white rounded-lg text-xs font-bold transition duration-300 flex items-center gap-1.5 shrink-0"
              title="Stop this step execution"
            >
              <XCircle className="w-3.5 h-3.5" />
              Stop Step
            </button>
          </div>
        )}

        {isStreamOpen && (
          <>
            {/* The buffer keeps a bounded tail (`lib/streamBuffer.ts`), so
                without this line a long turn's last 256 KB reads as everything
                the agent said. */}
            {streamTruncated && (
              <div className="mt-2 text-[10px] font-mono text-slate-500">
                Earlier output dropped — this is the tail of the turn.
              </div>
            )}
            <div className="mt-2 p-3 rounded-lg bg-[#020304] border border-cyan-500/20 max-h-64 overflow-y-auto font-mono text-[11px] shadow-inner flex flex-col-reverse">
              <pre className="text-cyan-300/80 whitespace-pre-wrap break-words">
                {stream || 'Waiting for agent output...'}
              </pre>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/**
 * Memoized, and every prop above is a primitive or a stable identity for that
 * reason: the timeline re-renders once per animation frame while an agent
 * streams, and the only card whose props changed is the one being streamed to.
 * A literal object, array or closure in any prop here re-opens that fan-out
 * (UI_REDESIGN_PLAN §4.2, §4.6).
 */
export const StepCard = memo(StepCardInner);
