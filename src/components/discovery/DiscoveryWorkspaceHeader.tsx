import React, { useEffect, useRef, useState } from 'react';
import { Code, Lock } from 'lucide-react';

import { setDiscoveryBase } from '../../lib/discovery';
import { baseBranchLabel, baseBranchLock, resolveBaseBranchInput } from '../../lib/discoveryBase';
import { discoveryLifecycle } from '../../lib/discoveryProgress';
import { formatError } from '../../lib/errors';
import { getRepositoriesForProject } from '../../lib/project';
import { listTerminalBranches } from '../../lib/terminal';
import { formatCost, formatTokens } from '../../lib/utils';
import type { Discovery, DiscoveryBoard, TerminalBranchOption } from '../../types';
import { BackButton } from '../ui/BackButton';
import { Chip } from '../ui/Chip';
import { FieldLabel } from '../ui/FieldLabel';
import { Metric, MetricStrip } from '../ui/MetricStrip';
import { OptionPill } from './OptionPill';

interface DiscoveryWorkspaceHeaderProps {
  discovery: Discovery;
  board: DiscoveryBoard | null;
  /** How many turns have been taken — stored messages, the same count Project
   *  Home's card reads. See `turnCountLabel` for why a turn is one message and
   *  not one rendered block. */
  turnCount: number;
  turnRunning: boolean;
  onToggleOpen: () => void;
  /** Raise the proposed-changes review (§5.1). The **user** decides when, and
   *  it is offered from the first turn — the interviewer's
   *  `nothing_left_to_settle` is advisory and never gates this button. */
  onDecompose: () => void;
  /** A pass is running. It streams through the interview's own events, so the
   *  transcript is already showing the work; this only stops a second press. */
  decomposing: boolean;
  busy: boolean;
  projectId: string;
  /** Called with the Discovery `setDiscoveryBase` returned, so the parent can
   *  fold the new `base_branch` into its state without a full `discovery_get`. */
  onBaseChanged: (discovery: Discovery) => void;
}

export function DiscoveryWorkspaceHeader({
  discovery,
  board,
  turnCount,
  turnRunning,
  onToggleOpen,
  onDecompose,
  decomposing,
  busy,
  projectId,
  onBaseChanged,
}: DiscoveryWorkspaceHeaderProps): React.ReactElement {
  const tickets = board?.tickets ?? [];
  const started = tickets.filter((view) => view.ticket.state === 'started').length;
  const lifecycle = discoveryLifecycle(discovery, tickets.length, turnRunning);
  const lock = baseBranchLock(tickets.map((view) => view.ticket));

  const [editingBase, setEditingBase] = useState(false);
  const [baseDraft, setBaseDraft] = useState<{ mode: 'default' | 'named'; name: string }>({
    mode: 'default',
    name: '',
  });
  const [baseSubmitting, setBaseSubmitting] = useState(false);
  const [baseError, setBaseError] = useState<string | null>(null);
  const [defaultBranchName, setDefaultBranchName] = useState<string | null>(null);
  const [branchOptions, setBranchOptions] = useState<TerminalBranchOption[]>([]);

  // Live host round trip this header otherwise never makes — deferred until
  // the user actually reaches for the edit control, mirroring
  // `NewDiscoveryModal`'s own load-on-open for the same data.
  const branchDataRequested = useRef(false);
  useEffect(() => {
    if (!editingBase || branchDataRequested.current) return;
    branchDataRequested.current = true;
    let cancelled = false;
    (async () => {
      try {
        const repos = await getRepositoriesForProject(projectId);
        const repositoryId = repos[0]?.id;
        if (!repositoryId) throw new Error('no repository');
        const branches = await listTerminalBranches(projectId, repositoryId);
        if (cancelled) return;
        setDefaultBranchName(branches.defaultBranch);
        setBranchOptions(branches.branches);
      } catch {
        if (cancelled) return;
        setDefaultBranchName(null);
        setBranchOptions([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [editingBase, projectId]);

  function openEditingBase() {
    setBaseDraft({
      mode: discovery.base_branch === null ? 'default' : 'named',
      name: discovery.base_branch ?? '',
    });
    setBaseError(null);
    setEditingBase(true);
  }

  function cancelEditingBase() {
    setEditingBase(false);
    setBaseError(null);
  }

  async function saveBase() {
    setBaseSubmitting(true);
    setBaseError(null);
    try {
      const branch = resolveBaseBranchInput(baseDraft.mode === 'default', baseDraft.name);
      const updated = await setDiscoveryBase(discovery.id, branch);
      onBaseChanged(updated);
      setEditingBase(false);
    } catch (err) {
      setBaseError(formatError(err));
    } finally {
      setBaseSubmitting(false);
    }
  }

  return (
    <header className="flex shrink-0 items-start justify-between gap-6 border-b border-white/5 bg-[#0d0f14]/60 px-6 py-3.5">
      <div className="flex min-w-0 items-start gap-3">
        <BackButton className="mt-1" />
        <div className="flex min-w-0 flex-col gap-1.5">
        <p className="m-0 font-mono text-[11px] text-slate-500">Discovery</p>
        <div className="flex min-w-0 items-center gap-3">
          <h1 className="m-0 truncate font-heading text-xl font-bold tracking-tight text-white">
            {discovery.title}
          </h1>
          <Chip size="sm" tone={lifecycle.tone} dot pulse={lifecycle.live}>
            {lifecycle.label}
          </Chip>
          {tickets.length > 0 && (
            <Chip size="sm" tone="slate">
              {tickets.length} tickets · {started} started
            </Chip>
          )}
        </div>

        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="truncate font-mono text-[11px] text-slate-500">
            {baseBranchLabel(discovery.base_branch, defaultBranchName)}
          </span>
          {!editingBase && (
            <button
              type="button"
              onClick={openEditingBase}
              disabled={lock.locked || busy}
              className="btn-secondary text-[11px] disabled:cursor-not-allowed disabled:opacity-40"
            >
              Change
            </button>
          )}
        </div>

        {lock.locked && (
          <p
            data-testid="discovery-base-locked"
            className="m-0 flex items-start gap-2 rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2.5 text-[11px] leading-relaxed text-slate-400"
          >
            <Lock className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-500" aria-hidden="true" />
            {lock.reason}
          </p>
        )}

        {editingBase && (
          <div className="nested-card mt-1 flex flex-col gap-2.5 px-3.5 py-3">
            <FieldLabel className="mb-0">Base branch</FieldLabel>
            <div role="radiogroup" aria-label="Base branch" className="flex flex-wrap gap-2">
              <OptionPill
                selected={baseDraft.mode === 'default'}
                onSelect={() => setBaseDraft((d) => ({ ...d, mode: 'default' }))}
              >
                Project default
              </OptionPill>
              <OptionPill
                selected={baseDraft.mode === 'named'}
                onSelect={() => setBaseDraft((d) => ({ ...d, mode: 'named' }))}
              >
                Named branch
              </OptionPill>
            </div>
            {baseDraft.mode === 'named' && (
              <div>
                <input
                  type="text"
                  value={baseDraft.name}
                  onChange={(e) => setBaseDraft((d) => ({ ...d, name: e.target.value }))}
                  list="discovery-header-base-branch-options"
                  placeholder="demeteo/features/my-integration-branch"
                  className="input-field"
                  aria-label="Base branch name"
                />
                <datalist id="discovery-header-base-branch-options">
                  {branchOptions.map((option) => (
                    <option key={option.name} value={option.name} />
                  ))}
                </datalist>
              </div>
            )}
            {baseError && (
              <p role="alert" className="m-0 font-mono text-[11px] text-ruby-200">
                {baseError}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={cancelEditingBase}
                disabled={baseSubmitting}
                className="btn-secondary text-[11px] disabled:cursor-not-allowed disabled:opacity-40"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => void saveBase()}
                disabled={
                  baseSubmitting || (baseDraft.mode === 'named' && baseDraft.name.trim().length === 0)
                }
                className="btn-primary text-[11px] disabled:cursor-not-allowed disabled:opacity-40"
              >
                {baseSubmitting ? 'Saving…' : 'Save'}
              </button>
            </div>
          </div>
        )}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-4">
        <MetricStrip variant="inset">
          <Metric label="Turns" value={String(turnCount)} />
          <Metric label="Spend" value={formatCost(discovery.total_cost)} tone="emerald" />
          <Metric label="Tokens" value={formatTokens(discovery.tokens)} tone="cyan" />
        </MetricStrip>

        <button type="button" onClick={onToggleOpen} disabled={busy} className="btn-secondary">
          {discovery.status === 'open' ? 'Close discovery' : 'Reopen discovery'}
        </button>

        <button
          type="button"
          data-testid="discovery-decompose"
          onClick={onDecompose}
          disabled={busy || decomposing}
          className="btn-primary inline-flex items-center gap-2 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Code className="h-3.5 w-3.5" aria-hidden="true" />
          {decomposing ? 'Decomposing…' : 'Decompose'}
        </button>
      </div>
    </header>
  );
}

export default DiscoveryWorkspaceHeader;
