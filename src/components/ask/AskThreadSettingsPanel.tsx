import React, { useEffect, useMemo, useState } from 'react';

import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { getAgentModels } from '../../lib/agentModels';
import { updateAskThreadSettings } from '../../lib/ask';
import { DEFAULT_EFFORT, EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import type { AskThread, ConfigOptionValue } from '../../types';
import { OptionPill } from '../discovery/OptionPill';
import { NetworkUnenforcedNote } from './NetworkUnenforcedNote';
import { Chip } from '../ui/Chip';
import { FieldLabel } from '../ui/FieldLabel';
import { Modal } from '../ui/Modal';

type SettingsPatch = Parameters<typeof updateAskThreadSettings>[1];

interface AskThreadSettingsPanelProps {
  thread: AskThread;
  onClose: () => void;
  onSaved: (thread: AskThread) => void;
}

/**
 * "Thread settings" panel (`docs/ask-canvas/probe/WebAccess.html`). A
 * thread's harness is fixed at creation — `AskThreadPatch` carries no
 * `agent_kind`, matching `DiscoveryPatch`'s existing precedent — so Agent
 * renders as a label here, never a picker. Model, effort and network are
 * the only fields this panel can change.
 */
export function AskThreadSettingsPanel({
  thread,
  onClose,
  onSaved,
}: AskThreadSettingsPanelProps): React.ReactElement {
  const { agents } = useAgentCatalog();
  const [model, setModel] = useState(thread.model ?? '');
  const [effort, setEffort] = useState<EffortLevel>(thread.effort ?? DEFAULT_EFFORT);
  const [network, setNetwork] = useState(thread.network);
  const [models, setModels] = useState<ConfigOptionValue[]>([]);
  const [modelsLoading, setModelsLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setModelsLoading(true);
    getAgentModels(thread.machine_id, thread.agent_kind)
      .then((list) => {
        if (!cancelled) setModels(list ?? []);
      })
      .catch(() => {
        if (!cancelled) setModels([]);
      })
      .finally(() => {
        if (!cancelled) setModelsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [thread.machine_id, thread.agent_kind]);

  const effortLevels = useMemo(
    () => effortLevelsFor(agents, thread.agent_kind),
    [agents, thread.agent_kind],
  );
  const effortSupported = effortLevels.length > 0;

  const save = async () => {
    setSubmitting(true);
    setError(null);
    try {
      const patch: SettingsPatch = {};
      if (model !== (thread.model ?? '')) patch.model = model || null;
      if (effortSupported && effort !== thread.effort) patch.effort = effort;
      if (network !== thread.network) patch.network = network;
      const updated = await updateAskThreadSettings(thread.id, patch);
      onSaved(updated);
      onClose();
    } catch (err) {
      setError(formatError(err));
      setSubmitting(false);
    }
  };

  return (
    <Modal onClose={onClose} className="w-full max-w-[640px] px-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Thread settings"
        className="glass-panel flex max-h-[85vh] flex-col overflow-hidden"
      >
        <div className="flex shrink-0 items-center justify-between border-b border-white/5 px-5 py-[18px]">
          <h2 className="font-heading text-[15px] font-semibold text-white">Thread settings</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="text-slate-500 transition-colors hover:text-white"
          >
            <svg
              viewBox="0 0 24 24"
              className="h-4 w-4"
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
              aria-hidden="true"
            >
              <path d="M18 6 6 18M6 6l12 12" strokeLinecap="round" />
            </svg>
          </button>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-[18px] overflow-y-auto p-5">
          <div>
            <FieldLabel>Agent</FieldLabel>
            <div data-testid="ask-settings-agent">
              <Chip tone="cyan">{thread.agent_kind}</Chip>
            </div>
            <p className="mt-2 text-[11px] text-slate-500">
              Fixed for this thread — set once, when it was created.
            </p>
          </div>

          <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
            <div>
              <FieldLabel>Model</FieldLabel>
              {modelsLoading ? (
                <p className="text-[11px] text-slate-500">Probing models…</p>
              ) : models.length === 0 ? (
                <p className="text-[11px] text-slate-500">
                  This agent lists no models here. It runs on its own default.
                </p>
              ) : (
                <div role="radiogroup" aria-label="Model" className="flex flex-wrap gap-2">
                  {models.map((option) => (
                    <OptionPill
                      key={option.value}
                      selected={option.value === model}
                      onSelect={() => setModel(option.value)}
                    >
                      {option.value}
                    </OptionPill>
                  ))}
                </div>
              )}
            </div>

            <div>
              <FieldLabel>Effort</FieldLabel>
              {effortSupported ? (
                <div role="radiogroup" aria-label="Effort" className="flex flex-wrap gap-2">
                  {effortLevels.map((level) => (
                    <OptionPill
                      key={level}
                      selected={level === effort}
                      onSelect={() => setEffort(level)}
                    >
                      {EFFORT_LABELS[level]}
                    </OptionPill>
                  ))}
                </div>
              ) : (
                <p className="text-[11px] text-slate-500">
                  {thread.agent_kind} exposes no per-invocation reasoning effort.
                </p>
              )}
            </div>
          </div>

          <div>
            <FieldLabel>Capability</FieldLabel>
            <div className="flex items-start gap-3 rounded-xl border border-white/5 bg-white/[0.02] px-3.5 py-3">
              <div className="min-w-0 flex-1">
                <p className="text-[13px] font-medium text-slate-100">
                  Read, run, reach the network — write denied
                </p>
                <p className="mt-1 text-[11.5px] leading-relaxed text-slate-500">
                  An Ask thread reads the repo and runs commands in its own worktree, and reaches
                  the network while Web access below is on. It writes nothing — no branch, no
                  commit.
                </p>
              </div>
            </div>
          </div>

          <div>
            <FieldLabel>Web access</FieldLabel>
            <div className="flex items-start gap-3.5 rounded-xl border border-emerald-500/25 bg-emerald-500/[0.06] p-3.5">
              <button
                type="button"
                role="switch"
                aria-checked={network}
                aria-label="Web access"
                data-testid="ask-settings-network-toggle"
                onClick={() => setNetwork((v) => !v)}
                className={`relative mt-0.5 h-[22px] w-[38px] shrink-0 rounded-full border transition-colors ${
                  network ? 'border-emerald-500/50 bg-emerald-500/35' : 'border-white/10 bg-white/10'
                }`}
              >
                <span
                  className={`absolute top-0.5 h-4 w-4 rounded-full transition-all ${
                    network
                      ? 'left-[18px] bg-emerald-400 shadow-[0_0_10px_rgba(16,185,129,0.7)]'
                      : 'left-0.5 bg-slate-400'
                  }`}
                />
              </button>
              <div className="min-w-0 flex-1">
                <p className="text-[13px] font-medium text-slate-100">
                  {network ? (
                    <>
                      On &mdash;{' '}
                      <span className="font-mono text-[12px] text-emerald-400">
                        network: Allow
                      </span>
                    </>
                  ) : (
                    <>
                      Off &mdash;{' '}
                      <span className="font-mono text-[12px] text-slate-400">network: Deny</span>
                    </>
                  )}
                </p>
                <p
                  data-testid="ask-settings-network-copy"
                  className="mt-1 text-[11.5px] leading-relaxed text-slate-500"
                >
                  {network
                    ? 'The agent may fetch documentation and upstream sources while it answers. Sources in the answer lists the distinct URLs it fetched while the turn runs; a web search is counted with the reads and is not listed. Once the turn settles only the counts remain.'
                    : 'The agent answers from the repo and what it already knows. A fetch is refused, so no Sources appear in the answer.'}
                </p>
                {thread.agent_kind === 'hermes' && <NetworkUnenforcedNote />}
              </div>
            </div>
          </div>

          {error && (
            <p role="alert" className="font-mono text-[11px] text-ruby-200">
              {error}
            </p>
          )}
        </div>

        <div className="flex shrink-0 justify-end gap-2.5 border-t border-white/5 bg-[#0d0f14]/90 px-5 py-4">
          <button type="button" onClick={onClose} className="btn-secondary text-[13px]">
            Cancel
          </button>
          <button
            type="button"
            onClick={save}
            disabled={submitting}
            className="btn-primary text-[13px] disabled:cursor-not-allowed disabled:opacity-40"
          >
            Save for this thread
          </button>
        </div>
      </div>
    </Modal>
  );
}

export default AskThreadSettingsPanel;
