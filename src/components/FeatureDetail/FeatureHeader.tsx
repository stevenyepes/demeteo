import { Cpu, GitBranch, GitPullRequest, RefreshCw, Terminal } from 'lucide-react';
import type { Project, RemoteRunMirror } from '../../types';
import { runStatusMeta, TERMINAL_STATUSES, TONE_CHIP } from '../../lib/runStatus';
import { formatCost, formatTokens } from '../../lib/utils';

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
  mrUrl: string | null;
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
  mrUrl,
  onBack,
  onOpenTerminalTab,
  onBrowseCode,
  onCancelFeature,
  onSync,
  onPublish,
  onCleanup,
}: FeatureHeaderProps) {
  return (
    <div className="p-6 border-b border-white/5 bg-[#0d0f14]/80 flex flex-wrap items-start justify-between gap-x-6 gap-y-4 backdrop-blur-md">
      <div className="space-y-1 min-w-0 flex-1">
        <div className="flex items-center gap-3 min-w-0">
          <button
            onClick={onBack}
            className="text-xs px-2.5 py-1 bg-white/5 hover:bg-white/10 rounded text-slate-400 hover:text-white transition uppercase font-bold shrink-0"
          >
            Back
          </button>
          <h1 className="text-xl font-bold font-display text-white tracking-wide line-clamp-2 break-words min-w-0 flex-1" title={featureTitle}>{featureTitle}</h1>
          <span
            className={`shrink-0 text-xs px-2.5 py-0.5 rounded-full font-bold uppercase border tracking-wider ${
              TONE_CHIP[statusMeta.tone]
            } ${statusMeta.active ? 'animate-pulse' : ''}`}
          >
            {statusMeta.label}
          </span>
          {/* Transport badge: where this run executes. Detached runs
              (mirror-listed) pulse while the 3s poll live-tails them;
              attached-remote is a project-level fact; everything else
              is a plain local run. */}
          <span
            className={`shrink-0 text-xs px-2.5 py-0.5 rounded-full font-bold uppercase border tracking-wider flex items-center gap-1 ${
              remoteRun || currentProject?.compute_type === 'remote'
                ? 'bg-cyan-500/10 text-cyan-400 border-cyan-500/20'
                : 'bg-white/5 text-slate-500 border-white/10'
            } ${remoteRun && !TERMINAL_STATUSES.includes(remoteRun.status) ? 'animate-pulse' : ''}`}
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
            <Cpu className="w-3 h-3" />
            {remoteRun
              ? 'Remote · Detached'
              : currentProject?.compute_type === 'remote'
              ? 'Remote · SSH'
              : 'Local'}
          </span>
        </div>
        <p className="text-xs text-slate-400 truncate">ID: {featureId}</p>
      </div>

      <div className="flex min-w-0 flex-col items-end gap-3">
        <div className="flex flex-wrap items-center justify-end gap-x-6 gap-y-2">
          <div className="text-right">
            <div className="text-[10px] text-slate-500 uppercase font-bold">Elapsed Duration</div>
            <div className="text-lg font-bold font-mono text-white">{duration}</div>
          </div>
          <div className="text-right">
            <div className="text-[10px] text-slate-500 uppercase font-bold">Pipeline Cost</div>
            <div className="text-lg font-bold font-mono text-emerald-400" title={`${totalCost.toFixed(4)} USD across ${stepCount} steps`}>
              {formatCost(totalCost)}
            </div>
          </div>
          <div className="text-right">
            <div className="text-[10px] text-slate-500 uppercase font-bold">Pipeline Tokens</div>
            <div className="text-lg font-bold font-mono text-cyan-400">{formatTokens(tokens)}</div>
          </div>
          {cacheReadTokens > 0 && (
            <div className="text-right">
              <div className="text-[10px] text-slate-500 uppercase font-bold">Cache Reads</div>
              <div
                className="text-lg font-bold font-mono text-violet-400"
                title={`${cacheReadTokens.toLocaleString()} tokens served from prompt cache (billed at ~10% of base input price) across this pipeline. ${cacheCreationTokens.toLocaleString()} tokens written to cache.`}
              >
                {formatTokens(cacheReadTokens)}
              </div>
            </div>
          )}
        </div>
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
                disabled={syncing || resolving}
                className="px-4 py-2 bg-cyan-600/20 hover:bg-cyan-600 border border-cyan-500/30 text-cyan-400 hover:text-white rounded-lg text-xs font-bold transition duration-300 disabled:opacity-40 flex items-center gap-1.5"
                title="Merge origin/main into this feature branch (resolves conflicts with a fresh agent when needed)"
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
