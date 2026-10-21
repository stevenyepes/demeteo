import { Code2, Cpu, ExternalLink, GitBranch, GitPullRequest, RefreshCw, Terminal } from 'lucide-react';
import type { FeatureDrift, MrState, Project, RemoteRunMirror } from '../../types';
import { runStatusMeta, TERMINAL_STATUSES } from '../../lib/runStatus';
import { describeStaleness, REFRESH_HINT } from '../../lib/staleness';
import { formatCost, formatTokens } from '../../lib/utils';
import { BackButton } from '../ui/BackButton';
import { Button, buttonClasses } from '../ui/Button';
import { Chip } from '../ui/Chip';
import { Metric, MetricStrip } from '../ui/MetricStrip';
import { OverflowMenu, type OverflowMenuItem } from '../ui/OverflowMenu';

interface FeatureHeaderProps {
  featureId: string;
  featureTitle: string;
  status: string;
  statusMeta: ReturnType<typeof runStatusMeta>;
  currentProject: Project | null;
  remoteRun: RemoteRunMirror | null;
  remoteMachineName: string | null;
  duration: string;
  totalCost: number;
  tokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  stepCount: number;
  publishing: boolean;
  /** What the Sync pane is holding for a human, if anything. > 0 is shown on
   *  the button; the pane itself says what it is. */
  syncBadge: number;
  /** How far behind the base a sync would merge, or `null` before a reading
   *  lands. The chip it produces is the answer to "why would I press Sync", and
   *  an unmeasurable branch renders as unknown rather than as current. */
  drift: FeatureDrift | null;
  /** A fetch of the base ref is in flight, asked for from this header. */
  driftRefreshing?: boolean;
  mrUrl: string | null;
  /** The pull request's state as last read from the provider, shown beside
   *  its link. The row that used to carry it under the header is gone. */
  mrState?: MrState | null;
  onRefreshMrState?: () => void;
  /** Quieter chrome for a scrolled run column; `lib/headerCollapse.ts` decides it. */
  collapsed?: boolean;
  onOpenTerminalTab: () => void;
  onBrowseCode: () => void;
  onCancelFeature: () => void;
  /** Select the Sync pane. This press starts nothing: every sync, resolve and
   *  publish now lives in that one pane, so the rules for when each is offered
   *  are spelled once, where the buttons are. */
  onOpenSync: () => void;
  /** False once the pull request is merged or closed: a merge into this
   *  branch can no longer reach one, so the header stops offering it. */
  syncOffered?: boolean;
  onPublish: () => void;
  onCleanup: () => void;
  /** Fetch `origin/<base>` and count again. This is the only press in the app
   *  that moves that ref for a finished feature — a run's bootstrap and a sync
   *  are the only other things that fetch it, and neither happens while a
   *  published pull request sits waiting — so without it the chip would answer
   *  from whatever an unrelated git flow last left behind. */
  onRefreshDrift?: () => void;
}

/**
 * Two lines: identity, status and actions on the first; telemetry on the
 * second. It was a two-column block whose right half stacked a four-tile
 * metric card over five buttons, which at half a window wrapped into three
 * rows and pushed the run down by the height of a card.
 *
 * Actions are ranked rather than listed. One primary — the thing a finished
 * run is waiting on (open the PR, or publish one) — carries the only filled
 * colour; the worktree tools are icon buttons; the rare ones (cleanup, refresh
 * the PR state, copy the id) sit in an overflow menu. Five filled buttons in
 * five tones read as five equally urgent things, and §4 of AGENTS.md gives
 * those tones meanings no button here has.
 *
 * Collapsed is the same header, quieter — half the vertical padding, one title
 * size step, and no id. Status, transport, telemetry and the actions are the
 * reason someone scrolls back up to this, so they stay where they were, which
 * makes the change a restyle rather than a remount. The transition stays on
 * padding: animating `box-shadow` or `scale` on a header like this one cost a
 * WKWebView GPU incident already — src/App.css records it above `pulse-glow`.
 */
export function FeatureHeader({
  featureId,
  featureTitle,
  status,
  statusMeta,
  currentProject,
  remoteRun,
  remoteMachineName,
  duration,
  totalCost,
  tokens,
  cacheReadTokens,
  cacheCreationTokens,
  stepCount,
  publishing,
  syncBadge,
  drift,
  driftRefreshing = false,
  mrUrl,
  mrState = null,
  onRefreshMrState,
  collapsed = false,
  onOpenTerminalTab,
  onBrowseCode,
  onCancelFeature,
  onOpenSync,
  syncOffered = true,
  onPublish,
  onCleanup,
  onRefreshDrift,
}: FeatureHeaderProps) {
  const staleness = describeStaleness(drift);
  const finished =
    status === 'completed' || status === 'failed' || status === 'cancelled' || status === 'awaiting_mr';
  const live = status === 'running' || status === 'verifying';

  const overflow: OverflowMenuItem[] = [];
  if (mrUrl && onRefreshMrState) {
    overflow.push({ label: 'Refresh PR state', onSelect: onRefreshMrState, title: 'Refresh MR state from the provider' });
  }
  if (finished || status === 'gated') {
    overflow.push({
      label: 'Cleanup',
      onSelect: () => onCleanup(),
      title:
        status === 'gated'
          ? "Apply the project's feature_lifecycle (archive / keep / auto_delete). Useful when a feature is stuck at a gate with a failed earlier step."
          : "Apply the project's feature_lifecycle (archive / keep / auto_delete)",
    });
  }
  overflow.push({
    label: 'Copy feature ID',
    onSelect: () => void navigator.clipboard?.writeText(featureId),
    title: featureId,
  });

  return (
    <div
      data-testid="feature-header"
      className={`px-6 ${
        collapsed ? 'py-3' : 'py-4'
      } border-b border-white/5 bg-surface-sidebar flex flex-col gap-1.5 transition-[padding] duration-200 ease-out motion-reduce:transition-none`}
    >
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <BackButton />
          <h1
            className={`${
              collapsed ? 'text-lg' : 'text-xl'
            } min-w-[8rem] shrink truncate font-bold font-heading text-white tracking-wide transition-[font-size] duration-200 ease-out motion-reduce:transition-none`}
            title={featureTitle}
          >
            {featureTitle}
          </h1>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            <Chip status={status} tone={statusMeta.tone} pulse={statusMeta.active}>
              {statusMeta.label}
            </Chip>
            {/* Transport badge: where this run executes. A detached run is live
                while its mirror is non-terminal; attached-remote is a
                project-level fact; everything else is a plain local run.
                Cyan for either remote flavour, slate for local — a transport is
                not a run status, so it takes a tone directly rather than
                resolving one. */}
            <Chip
              tone={remoteRun || currentProject?.compute_type === 'remote' ? 'cyan' : 'slate'}
              icon={<Cpu className="w-3 h-3" />}
              pulse={remoteRun !== null && !TERMINAL_STATUSES.includes(remoteRun.status)}
              title={
                remoteRun
                  ? `Detached run on ${remoteMachineName ?? remoteRun.machine_id}${
                      TERMINAL_STATUSES.includes(remoteRun.status) ? '' : ' — live'
                    }`
                  : currentProject?.compute_type === 'remote'
                  ? `Executes on ${currentProject.remote_host ?? 'the project machine'} over SSH, orchestrated by this app`
                  : 'Executes on this machine'
              }
            >
              {remoteRun
                ? 'Remote · Detached'
                : currentProject?.compute_type === 'remote'
                ? 'Remote · SSH'
                : 'Local'}
            </Chip>
            {staleness && (
              <button
                type="button"
                onClick={onRefreshDrift}
                disabled={!onRefreshDrift || driftRefreshing}
                data-testid="drift-refresh"
                className="whitespace-nowrap rounded-full transition disabled:cursor-default enabled:hover:brightness-125"
                title={
                  onRefreshDrift
                    ? `${staleness.title} ${driftRefreshing ? 'Fetching…' : REFRESH_HINT}`
                    : staleness.title
                }
              >
                <Chip
                  tone={staleness.tone}
                  dot={false}
                  icon={driftRefreshing ? <RefreshCw className="w-3 h-3 animate-spin" /> : undefined}
                >
                  {staleness.label}
                </Chip>
              </button>
            )}
            {mrUrl && (
              <a
                href={mrUrl}
                target="_blank"
                rel="noopener noreferrer"
                data-testid="header-pr-link"
                title={mrUrl}
                className="flex items-center gap-1 whitespace-nowrap font-mono text-xs text-accent-ink transition hover:text-white"
              >
                <GitPullRequest className="h-3.5 w-3.5" />
                PR {mrState ?? 'unknown'}
                <ExternalLink className="h-3 w-3" />
              </a>
            )}
          </div>
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
          {live && (
            <Button variant="danger-subtle" onClick={onCancelFeature} className="h-8">
              Cancel Feature
            </Button>
          )}
          {finished && syncOffered && (
            <Button
              onClick={onOpenSync}
              data-testid="open-sync"
              className="h-8"
              title="Show this branch's sync: how far behind it is, any conflict, and what to do about it"
            >
              <GitBranch className="w-3.5 h-3.5" />
              {syncBadge > 0 ? `Sync · ${syncBadge}` : 'Sync'}
            </Button>
          )}
          <button
            onClick={onOpenTerminalTab}
            aria-label="Code with Agent"
            title="Code with Agent — open an interactive agent coding session in this feature's worktree"
            className="flex h-8 w-8 items-center justify-center rounded-lg border border-white/10 bg-white/[0.03] text-slate-400 transition hover:bg-white/[0.08] hover:text-white"
          >
            <Terminal className="w-4 h-4" />
          </button>
          <button
            onClick={onBrowseCode}
            aria-label="Browse Code"
            title="Browse Code — the feature branch, read-only"
            className="flex h-8 w-8 items-center justify-center rounded-lg border border-white/10 bg-white/[0.03] text-slate-400 transition hover:bg-white/[0.08] hover:text-white"
          >
            <Code2 className="w-4 h-4" />
          </button>
          <OverflowMenu label="More actions" items={overflow} />
          {/* The finalize step opens the PR itself at the end of a run, so once
              there is a URL the only useful action is to go look at it.
              Publishing by hand stays available for features whose run never
              produced one. */}
          {finished &&
            (mrUrl ? (
              <a
                href={mrUrl}
                target="_blank"
                rel="noopener noreferrer"
                className={`${buttonClasses('primary')} h-8`}
                title="Open the pull request in your browser"
              >
                <GitPullRequest className="w-3.5 h-3.5" />
                View PR
              </a>
            ) : (
              <Button
                variant="primary"
                onClick={onPublish}
                disabled={publishing}
                className="h-8"
                title="Open a PR/MR for review. The title and description are written by the agent; there is nothing to fill in."
              >
                {publishing ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <GitPullRequest className="w-3.5 h-3.5" />}
                Publish MR
              </Button>
            ))}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-x-5 gap-y-1 pl-11">
        <MetricStrip variant="line">
          <Metric inline label="Elapsed" value={duration} />
          <Metric
            inline
            label="Cost"
            value={formatCost(totalCost)}
            tone="emerald"
            tooltip={`${totalCost.toFixed(4)} USD across ${stepCount} steps`}
          />
          <Metric inline label="Tokens" value={formatTokens(tokens)} tone="cyan" />
          {cacheReadTokens > 0 && (
            <Metric
              inline
              label="Cache Reads"
              value={formatTokens(cacheReadTokens)}
              tone="slate"
              tooltip={`${cacheReadTokens.toLocaleString()} tokens served from prompt cache (billed at ~10% of base input price) across this pipeline. ${cacheCreationTokens.toLocaleString()} tokens written to cache.`}
            />
          )}
        </MetricStrip>
        {!collapsed && <span className="truncate font-mono text-xs text-slate-600">ID: {featureId}</span>}
      </div>
    </div>
  );
}
