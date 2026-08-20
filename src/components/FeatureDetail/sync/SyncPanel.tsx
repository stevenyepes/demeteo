import type { ReactNode } from 'react';
import {
  Cpu,
  GitBranch,
  GitCompare,
  GitMerge,
  RefreshCw,
  Radio,
  Trash2,
  Upload,
  XCircle,
} from 'lucide-react';

import { TONE_TEXT } from '../../../lib/runStatus';
import { isReadOnlySyncIntent, type SyncAction, type SyncIntent, type SyncPanelModel } from '../../../lib/syncPanel';
import { formatCost, formatDuration, formatTokens, relativeTime } from '../../../lib/utils';
import type { FeatureDrift, StepExecution, SyncSessionView } from '../../../types';
import { Chip } from '../../ui/Chip';
import { INSPECTOR_SURFACE } from '../../ui/Inspector';
import { Metric, MetricStrip } from '../../ui/MetricStrip';
import { ActionRow } from '../../ui/ActionRow';
import { SyncResolverOptions } from '../SyncResolverOptions';
import type { SyncResolverSelection } from '../useSyncResolverOverrides';
import { ConflictFileList } from './ConflictFileList';

/**
 * Everything one feature branch's sync is doing, in one pane.
 *
 * It replaces five surfaces that grew one per phase — a result banner, an abort
 * button on one of its branches, a harness picker, a review card and a
 * staleness chip — each correct alone and, stacked, a strip of notices with no
 * single place that answers "what is happening with this branch". The chip on
 * the header survives as the entry point; everything else is here.
 *
 * The component decides nothing. `lib/syncPanel.ts` turns the durable session
 * row into what to say and what to offer, including which affordances the user
 * owns, so the whole policy is reachable from a test with no DOM. What is left
 * here is which glyph an intent wears and where the copy sits.
 *
 * It is deliberately **not** an `Inspector`: that shell requires a tab strip,
 * and a fifth tab beside Overview / Live / Output / Actions would claim to
 * describe the selected step. It borrows `INSPECTOR_SURFACE` so the two panes
 * are the same card, which is the only thing they share.
 */
export interface SyncPanelProps {
  model: SyncPanelModel;
  session: SyncSessionView | null;
  drift: FeatureDrift | null;
  /** The resolver's own `step_executions` row while one is running — the same
   *  telemetry every other step reports, read from the same place. */
  resolverStep: StepExecution | null;
  /** Which action is in flight. One value, not a boolean per intent: two of
   *  these can never overlap, and separate flags let a render show both. */
  pending: SyncIntent | null;
  /** Absent where nobody holds a picker — the model also has to want one. */
  resolverSelection?: SyncResolverSelection;
  onAction: (intent: SyncIntent) => void;
  onOpenPath: (filePath: string) => void;
  className?: string;
}

const ACTION_ICON: Record<SyncIntent, ReactNode> = {
  sync: <GitBranch className="h-4 w-4" />,
  resolve: <Cpu className="h-4 w-4" />,
  abort: <XCircle className="h-4 w-4" />,
  review: <GitCompare className="h-4 w-4" />,
  publish: <Upload className="h-4 w-4" />,
  discard: <Trash2 className="h-4 w-4" />,
  refresh: <RefreshCw className="h-4 w-4" />,
  watch: <Radio className="h-4 w-4" />,
};

const PENDING_LABEL: Record<SyncIntent, string> = {
  sync: 'Syncing…',
  resolve: 'Resolving…',
  abort: 'Aborting…',
  review: 'Opening…',
  publish: 'Publishing…',
  discard: 'Discarding…',
  refresh: 'Counting…',
  watch: 'Opening…',
};

export function SyncPanel({
  model,
  session,
  drift,
  resolverStep,
  pending,
  resolverSelection,
  onAction,
  onOpenPath,
  className = '',
}: SyncPanelProps) {
  return (
    <div
      data-testid="sync-panel"
      data-sync-state={model.state}
      className={`${INSPECTOR_SURFACE} ${className}`}
    >
      <div className="flex shrink-0 items-start justify-between gap-3 border-b border-white/5 px-5 py-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <GitMerge className={`h-4 w-4 shrink-0 ${TONE_TEXT[model.tone]}`} />
            <h3 className="truncate font-heading text-sm font-bold uppercase tracking-wider text-white">
              Branch sync
            </h3>
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-2">
            <Chip tone={model.tone} pulse={model.live} dot>
              {model.chipLabel}
            </Chip>
            {model.branch && (
              <Chip tone="slate" dot={false} title={`${model.baseBranch} → ${model.branch}`}>
                {model.branch}
              </Chip>
            )}
          </div>
        </div>
      </div>

      <div
        key={model.state}
        className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4 animate-fade-in"
      >
        <div>
          <h4 className="font-heading text-sm font-bold text-white">{model.headline}</h4>
          <p className="mt-1 text-xs leading-relaxed text-slate-400">{model.body}</p>
        </div>

        <SyncFacts drift={drift} session={session} resolverStep={resolverStep} />

        {model.conflictFiles.length > 0 || model.state === 'conflicted' ? (
          <Section title="Unmerged paths">
            <ConflictFileList files={model.conflictFiles} onOpenPath={onOpenPath} />
          </Section>
        ) : null}

        {model.detail && (
          <Section title="What git said">
            <pre
              data-testid="sync-raw-error"
              className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-xl border border-white/5 bg-black/30 p-3 font-mono text-[11px] leading-relaxed text-slate-300"
            >
              {model.detail}
            </pre>
          </Section>
        )}

        {model.showResolver && resolverSelection && (
          <div className="rounded-xl border border-white/5 bg-black/20 p-3.5">
            <div className="mb-2.5 text-[10px] font-bold uppercase tracking-widest text-slate-500">
              Resolve with
            </div>
            <SyncResolverOptions selection={resolverSelection} />
          </div>
        )}

        {model.actions.length > 0 && (
          <div className="space-y-3">
            {model.actions.map((action) => (
              <SyncActionRow
                key={action.intent}
                action={action}
                pending={pending}
                onAction={onAction}
              />
            ))}
          </div>
        )}

        {model.worktreePath && (
          <Section title="Sync worktree">
            <p
              data-testid="sync-worktree-path"
              className="select-all break-all rounded-lg border border-white/5 bg-black/30 px-3 py-2 font-mono text-[11px] text-slate-400"
            >
              {model.worktreePath}
            </p>
          </Section>
        )}
      </div>
    </div>
  );
}

function SyncActionRow({
  action,
  pending,
  onAction,
}: {
  action: SyncAction;
  pending: SyncIntent | null;
  onAction: (intent: SyncIntent) => void;
}) {
  const busy = pending !== null && !isReadOnlySyncIntent(action.intent);
  return (
    <ActionRow
      icon={ACTION_ICON[action.intent]}
      tone={action.tone}
      title={action.label}
      desc={action.desc}
      buttonLabel={pending === action.intent ? PENDING_LABEL[action.intent] : action.label}
      buttonTitle={action.title}
      onClick={() => onAction(action.intent)}
      disabled={busy}
      disabledReason="Another sync action is still running."
    />
  );
}

/**
 * The counts, and where they came from.
 *
 * `fetched` travels with the drift reading precisely so a week-old number is
 * not presented as this minute's, so the label says which one it is rather than
 * the strip implying the expensive one.
 */
function SyncFacts({
  drift,
  session,
  resolverStep,
}: {
  drift: FeatureDrift | null;
  session: SyncSessionView | null;
  resolverStep: StepExecution | null;
}) {
  const behind = drift?.divergence.behind ?? null;
  const ahead = drift?.divergence.ahead ?? null;
  const showDrift = drift !== null;
  const showResolver = resolverStep !== null;
  if (!showDrift && !showResolver && session === null) return null;

  return (
    <MetricStrip variant="inset" className="w-full">
      {showDrift && (
        <Metric
          label={drift.fetched ? 'Behind' : 'Behind (cached)'}
          value={behind === null ? '—' : String(behind)}
          tone={behind === null ? 'slate' : behind > 0 ? 'cyan' : 'emerald'}
          tooltip={
            drift.fetched
              ? `Counted against ${drift.base_ref} just now.`
              : `Counted against ${drift.base_ref} as of the last time it was fetched.`
          }
        />
      )}
      {showDrift && <Metric label="Ahead" value={ahead === null ? '—' : String(ahead)} />}
      {showResolver && (
        <Metric label="Cost" value={formatCost(resolverStep.cost_usd)} tone="emerald" />
      )}
      {showResolver && (
        <Metric label="Tokens" value={formatTokens(resolverStep.tokens)} tone="cyan" />
      )}
      {showResolver && (
        <Metric label="Elapsed" value={formatDuration(resolverStep.wall_clock_secs)} />
      )}
      {session !== null && (
        <Metric label="Attempts" value={String(session.attempts)} />
      )}
      {session !== null && (
        <Metric label="Updated" value={relativeTime(session.updated_at)} />
      )}
    </MetricStrip>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div>
      <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
        {title}
      </div>
      {children}
    </div>
  );
}

export default SyncPanel;
