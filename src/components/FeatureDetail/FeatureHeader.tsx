import { Cpu, GitBranch, GitPullRequest, RefreshCw, Terminal } from 'lucide-react';
import type { FeatureDrift, Project, RemoteRunMirror } from '../../types';
import { runStatusMeta, TERMINAL_STATUSES } from '../../lib/runStatus';
import { describeStaleness } from '../../lib/staleness';
import { formatCost, formatTokens } from '../../lib/utils';
import { Chip } from '../ui/Chip';
import { Metric, MetricStrip } from '../ui/MetricStrip';

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
  syncing: boolean;
  resolving: boolean;
  publishing: boolean;
  /** A resolution is committed on the branch and waiting to be published or
   *  discarded. Syncing again is refused while one is — the backend answers
   *  `held_resolution` — so the button says so here rather than sending a
   *  request that can only come back as a banner. */
  reviewHeld: boolean;
  /** How far behind the base a sync would merge, or `null` before a reading
   *  lands. The chip it produces is the answer to "why would I press Sync", and
   *  an unmeasurable branch renders as unknown rather than as current. */
  drift: FeatureDrift | null;
  mrUrl: string | null;
  /** Quieter chrome for a scrolled run column; `lib/headerCollapse.ts` decides it. */
  collapsed?: boolean;
  onBack: () => void;
  onOpenTerminalTab: () => void;
  onBrowseCode: () => void;
  onCancelFeature: () => void;
  onSync: () => void;
  onPublish: () => void;
  onCleanup: () => void;
}

/**
 * Title on the left, telemetry + actions on the right — one row while both
 * fit, stacked once they don't. Nothing here is `shrink-0` at the group
 * level: a run with every action available (sync / publish / cleanup) is
 * wider than a half-window, and it should wrap rather than push the header
 * off-screen.
 *
 * Collapsed is the same header, quieter — it gives back the id line, half the
 * vertical padding and one title size step, and nothing else. Status, transport,
 * telemetry and the actions are the reason someone scrolls back up to this, so
 * hiding them would trade one scroll for another; every element that survives
 * keeps its position, which makes the change a restyle rather than a remount.
 * The transition stays on padding: this element is `backdrop-blur-md` over a
 * translucent surface, and animating `box-shadow` or `scale` on one of those
 * cost a WKWebView GPU incident already — src/App.css records it above
 * `pulse-glow`.
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
  syncing,
  resolving,
  publishing,
  reviewHeld,
  drift,
  mrUrl,
  collapsed = false,
  onBack,
  onOpenTerminalTab,
  onBrowseCode,
  onCancelFeature,
  onSync,
  onPublish,
  onCleanup,
}: FeatureHeaderProps) {
  const staleness = describeStaleness(drift);

  return (
    <div
      data-testid="feature-header"
      className={`px-6 ${
        collapsed ? 'py-3' : 'py-6'
      } border-b border-white/5 bg-[#0d0f14]/80 flex flex-wrap items-start justify-between gap-x-6 gap-y-4 backdrop-blur-md transition-[padding] duration-200 ease-out motion-reduce:transition-none`}
    >
      <div className="space-y-1 min-w-0 flex-1">
        <div className="flex items-center gap-3 min-w-0">
          <button
            onClick={onBack}
            className="text-xs px-2.5 py-1 bg-white/5 hover:bg-white/10 rounded text-slate-400 hover:text-white transition uppercase font-bold shrink-0"
          >
            Back
          </button>
          <h1
            className={`${
              collapsed ? 'text-lg' : 'text-xl'
            } font-bold font-heading text-white tracking-wide line-clamp-2 break-words min-w-0 flex-1 transition-[font-size] duration-200 ease-out motion-reduce:transition-none`}
            title={featureTitle}
          >
            {featureTitle}
          </h1>
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
            <Chip tone={staleness.tone} dot={false} title={staleness.title}>
              {staleness.label}
            </Chip>
          )}
        </div>
        {!collapsed && <p className="text-xs text-slate-400 truncate">ID: {featureId}</p>}
      </div>

      <div className="flex min-w-0 flex-col items-end gap-3">
        <MetricStrip variant="inset" className="justify-end">
          <Metric label="Elapsed" value={duration} />
          <Metric
            label="Cost"
            value={formatCost(totalCost)}
            tone="emerald"
            tooltip={`${totalCost.toFixed(4)} USD across ${stepCount} steps`}
          />
          <Metric label="Tokens" value={formatTokens(tokens)} tone="cyan" />
          {cacheReadTokens > 0 && (
            <Metric
              label="Cache Reads"
              value={formatTokens(cacheReadTokens)}
              tone="violet"
              tooltip={`${cacheReadTokens.toLocaleString()} tokens served from prompt cache (billed at ~10% of base input price) across this pipeline. ${cacheCreationTokens.toLocaleString()} tokens written to cache.`}
            />
          )}
        </MetricStrip>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <button
            onClick={onOpenTerminalTab}
            className="px-4 py-2 bg-cyan-600/20 hover:bg-cyan-600 border border-cyan-500/30 text-cyan-300 hover:text-white rounded-lg text-xs font-bold transition duration-300 flex items-center gap-1.5"
            title="Open an interactive agent coding session in this feature's worktree"
          >
            <Terminal className="w-3.5 h-3.5" />
            Code with Agent
          </button>
          <button
            onClick={onBrowseCode}
            className="px-4 py-2 bg-violet-600/20 hover:bg-violet-600 border border-violet-500/30 text-violet-300 hover:text-white rounded-lg text-xs font-bold transition duration-300 flex items-center gap-1.5"
            title="Browse the feature branch code in read-only mode"
          >
            <GitBranch className="w-3.5 h-3.5" />
            Browse Code
          </button>
          {(status === 'running' || status === 'verifying') && (
            <button
              onClick={onCancelFeature}
              className="px-4 py-2 bg-rose-600/20 hover:bg-rose-600 border border-rose-500/30 text-rose-400 hover:text-white rounded-lg text-xs font-bold transition duration-300"
            >
              Cancel Feature
            </button>
          )}
          {(status === 'completed' || status === 'failed' || status === 'cancelled' || status === 'awaiting_mr') && (
            <>
              <button
                onClick={onSync}
                disabled={syncing || resolving || reviewHeld}
                className="px-4 py-2 bg-cyan-600/20 hover:bg-cyan-600 border border-cyan-500/30 text-cyan-400 hover:text-white rounded-lg text-xs font-bold transition duration-300 disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-1.5"
                title={
                  reviewHeld
                    ? 'A resolution from the last sync is still waiting to be published or discarded. Deal with it below, then sync again.'
                    : 'Merge origin/main into this feature branch (resolves conflicts with a fresh agent when needed)'
                }
              >
                {syncing ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <GitBranch className="w-3.5 h-3.5" />}
                Sync with main
              </button>
              {/* The finalize step opens the PR itself at the end of a run,
                  so once there is a URL the only useful action is to go
                  look at it. Publishing by hand stays available for features
                  whose run never produced one. */}
              {mrUrl ? (
                <a
                  href={mrUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="px-4 py-2 bg-emerald-600/20 hover:bg-emerald-600 border border-emerald-500/30 text-emerald-400 hover:text-white rounded-lg text-xs font-bold transition duration-300 flex items-center gap-1.5"
                  title="Open the pull request in your browser"
                >
                  <GitPullRequest className="w-3.5 h-3.5" />
                  View PR
                </a>
              ) : (
                <button
                  onClick={onPublish}
                  disabled={publishing}
                  className="px-4 py-2 bg-emerald-600/20 hover:bg-emerald-600 border border-emerald-500/30 text-emerald-400 hover:text-white rounded-lg text-xs font-bold transition duration-300 disabled:opacity-40 flex items-center gap-1.5"
                  title="Open a PR/MR for review. The title and description are written by the agent; there is nothing to fill in."
                >
                  {publishing ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <GitPullRequest className="w-3.5 h-3.5" />}
                  Publish MR
                </button>
              )}
              <button
                onClick={() => onCleanup()}
                className="px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 rounded-lg text-xs font-bold transition duration-300"
                title="Apply the project's feature_lifecycle (archive / keep / auto_delete)"
              >
                Cleanup
              </button>
            </>
          )}
          {status === 'gated' && (
            <button
              onClick={() => onCleanup()}
              className="px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 rounded-lg text-xs font-bold transition duration-300"
              title="Apply the project's feature_lifecycle (archive / keep / auto_delete). Useful when a feature is stuck at a gate with a failed earlier step."
            >
              Cleanup
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
