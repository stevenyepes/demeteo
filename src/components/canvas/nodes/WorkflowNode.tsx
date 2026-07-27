/**
 * The card React Flow draws for one workflow node. Dark neon glassmorphism to
 * match the rest of the app; the kind icon + type chip make a graph scannable
 * without opening panels (the anti-"identical boxes" rule, PRD §6.3).
 *
 * Run-mode overlay (P2.2): when `data.run` is present the card takes on the
 * run-status color language (`lib/runStatus.ts`) — a pulsing dot for in-motion
 * nodes, a tone-matched glow, duration+cost chips on completion, and the
 * failure class on a failed node. Animation is **opacity-only** (`animate-pulse`,
 * static box-shadows) to honor the webview battery rule; no infinite transforms.
 */
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { AlertTriangle, OctagonAlert, ShieldCheck } from 'lucide-react';
import { nodeTypeMeta } from '../types';
import type { EssenceKind } from '../nodeSummary';
import {
  runStatusMeta,
  TONE_TEXT,
  TONE_CHIP,
  type RunStatusTone,
} from '../../../lib/runStatus';
import { formatCost, formatDuration } from '../../../lib/utils';
import type { WorkflowFlowNode } from '../flowGraph';

const HANDLE_CLASS = '!h-2 !w-2 !border-slate-600 !bg-slate-800';

/** Card border + glow per run tone — the run-mode equivalent of the
 *  timeline's per-status card tint. Static box-shadows only. */
const TONE_CARD: Record<RunStatusTone, string> = {
  cyan: 'border-cyan-500/50 shadow-[0_0_18px_rgba(6,182,212,0.18)]',
  violet: 'border-violet-500/50 shadow-[0_0_18px_rgba(139,92,246,0.18)]',
  amber: 'border-amber-500/50 shadow-[0_0_18px_rgba(245,158,11,0.20)]',
  emerald: 'border-emerald-500/40 shadow-lg shadow-black/30',
  ruby: 'border-rose-500/50 shadow-[0_0_18px_rgba(244,63,94,0.22)]',
  slate: 'border-slate-700/60 shadow-lg shadow-black/30',
};

/** Config-essence chip tint per badge kind (design mode, P3.2). Muted on
 *  purpose: these are scanning aids, not status — the run overlay owns the
 *  loud colors. */
const ESSENCE_CHIP: Record<EssenceKind, string> = {
  agent: 'border-cyan-500/20 bg-cyan-500/10 text-cyan-300/90',
  model: 'border-violet-500/20 bg-violet-500/10 text-violet-300/90',
  effort: 'border-slate-600/40 bg-slate-700/20 text-slate-300',
  capability: 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300/90',
  flag: 'border-slate-600/40 bg-slate-800/40 text-slate-400',
};

/** Solid status dot per run tone. */
const TONE_DOT: Record<RunStatusTone, string> = {
  cyan: 'bg-cyan-400',
  violet: 'bg-violet-400',
  amber: 'bg-amber-400',
  emerald: 'bg-emerald-400',
  ruby: 'bg-rose-400',
  slate: 'bg-slate-500',
};

/**
 * Structural-lint badge (P3.3). Errors win: a node with both shows the ruby
 * octagon, because that is the one blocking the save. The messages are the
 * tooltip — the same strings the blocked-save list names, so the badge and the
 * bar can't tell different stories.
 */
function LintBadge({ errors, warnings }: { errors: string[]; warnings: string[] }) {
  const blocking = errors.length > 0;
  const messages = blocking ? errors : warnings;
  if (messages.length === 0) return null;
  const Icon = blocking ? OctagonAlert : AlertTriangle;
  return (
    <span
      title={messages.join('\n')}
      data-testid={blocking ? 'node-lint-error' : 'node-lint-warning'}
      className="shrink-0"
    >
      <Icon
        className={`h-3.5 w-3.5 ${blocking ? 'text-rose-400' : 'text-amber-400'}`}
        aria-label={
          blocking
            ? `${errors.length} lint error${errors.length === 1 ? '' : 's'}`
            : `${warnings.length} lint warning${warnings.length === 1 ? '' : 's'}`
        }
      />
    </span>
  );
}

export function WorkflowNode({ data, selected }: NodeProps<WorkflowFlowNode>) {
  const meta = nodeTypeMeta(data.nodeType);
  const Icon = meta.icon;
  const lint = data.lint;

  const run = data.run;
  const runMeta = run ? runStatusMeta(run.status) : null;
  const tone = runMeta?.tone ?? null;
  const isActive = run?.status === 'running' || run?.status === 'verifying';
  const isSkipped = run?.status === 'skipped';
  const isFailed = run?.status === 'failed' || run?.status === 'interrupted';
  const isDone = run?.status === 'completed';
  const showChips = isDone || isFailed;

  return (
    <div
      className={[
        'group flex min-w-[200px] max-w-[280px] flex-col gap-2 rounded-xl border px-3.5 py-2.5',
        'bg-slate-900/70 backdrop-blur-sm transition-shadow',
        isSkipped ? 'opacity-50' : '',
        // The replay cone reads as a violet "will re-run" ring, distinct from
        // the cyan selection highlight (P2.4). Selection still wins the border.
        data.highlighted && !selected
          ? 'border-violet-400/60 shadow-[0_0_0_2px_rgba(167,139,250,0.45),0_0_18px_rgba(139,92,246,0.25)]'
          : selected
            ? 'border-cyan-400/70 shadow-[0_0_0_1px_rgba(34,211,238,0.4),0_0_18px_rgba(34,211,238,0.25)]'
            : tone
              ? TONE_CARD[tone]
              : 'border-slate-700/60 shadow-lg shadow-black/30 hover:border-slate-600',
      ].join(' ')}
      title={
        isSkipped
          ? `Skipped${run?.errorClass ? `: ${run.errorClass.replace(/_/g, ' ')}` : ''}`
          : undefined
      }
    >
      <Handle type="target" position={Position.Top} className={HANDLE_CLASS} />

      <div className="flex items-center gap-3">
        <div
          className={[
            'relative flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border',
            TONE_CHIP[meta.tone],
          ].join(' ')}
        >
          <Icon className={`h-4 w-4 ${TONE_TEXT[meta.tone]}`} aria-hidden />
          {tone && (
            <span
              className={[
                'absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full ring-2 ring-slate-900',
                TONE_DOT[tone],
                isActive ? 'animate-pulse' : '',
              ].join(' ')}
              aria-hidden
            />
          )}
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <div className="truncate text-sm font-medium text-slate-100" title={data.title}>
              {data.title}
            </div>
            {lint && <LintBadge errors={lint.errors} warnings={lint.warnings} />}
          </div>
          <div
            className={`text-[11px] font-medium uppercase tracking-wide ${
              tone ? TONE_TEXT[tone] : TONE_TEXT[meta.tone]
            }`}
          >
            {runMeta ? runMeta.label : meta.label}
          </div>
        </div>
      </div>

      {showChips && (
        <div className="flex items-center gap-2 pl-11 text-[10px] font-mono">
          {isFailed && run?.errorClass && (
            <span className="rounded border border-rose-500/20 bg-rose-500/10 px-1.5 py-0.5 text-rose-300">
              {run.errorClass.replace(/_/g, ' ')}
            </span>
          )}
          {typeof run?.costUsd === 'number' && run.costUsd > 0 && (
            <span className="text-emerald-400">{formatCost(run.costUsd)}</span>
          )}
          {typeof run?.wallClockSecs === 'number' && run.wallClockSecs > 0 && (
            <span className="text-slate-400">{formatDuration(run.wallClockSecs)}</span>
          )}
        </div>
      )}

      {data.essence && (
        <div
          className="flex flex-wrap items-center gap-1 pl-11 text-[9px] font-mono"
          data-testid="node-essence"
        >
          {data.essence.verifier && (
            <span title="A verifier turn checks this node's output">
              <ShieldCheck className="h-3 w-3 text-emerald-400" aria-label="Verified" />
            </span>
          )}
          {data.essence.badges.map((b) => (
            <span
              key={`${b.kind}:${b.label}`}
              title={b.hint}
              className={`rounded border px-1 py-0.5 ${ESSENCE_CHIP[b.kind]}`}
            >
              {b.label}
            </span>
          ))}
          {data.essence.retry.slice(0, 2).map((r) => (
            <span
              key={r}
              title={`Retry: ${data.essence!.retry.join(' · ')}`}
              className="rounded border border-amber-500/20 bg-amber-500/10 px-1 py-0.5 text-amber-300/90"
            >
              {r}
            </span>
          ))}
        </div>
      )}

      <Handle type="source" position={Position.Bottom} className={HANDLE_CLASS} />
    </div>
  );
}
