import React, { useEffect, useMemo, useState } from 'react';

import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { getAgentModels } from '../../lib/agentModels';
import { createAskThread } from '../../lib/ask';
import { DEFAULT_EFFORT, EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import { listMachines } from '../../lib/machines';
import {
  interviewerMachineOptions,
  nameFieldState,
  TITLE_MAX_CHARS,
} from '../../lib/newDiscovery';
import type { AskThread, ConfigOptionValue, Machine } from '../../types';
import { FieldLabel } from '../ui/FieldLabel';
import { Modal } from '../ui/Modal';
import { OptionPill } from '../discovery/OptionPill';
import { NetworkUnenforcedNote } from './NetworkUnenforcedNote';

/** What the dimmed group shows when the harness has no effort control at all. */
const DIMMED_EFFORTS: readonly EffortLevel[] = ['low', 'medium', 'high'];

interface NewAskThreadModalProps {
  projectId: string;
  /** The project's own host, which is where its repository was cloned. */
  machineId: string;
  /** Names the eyebrow label above the modal title; falls back to a generic
   *  string when the caller has none to give. */
  projectName?: string;
  /** A "Try" chip's text, carried in as the starting name so the question the
   *  user picked is not retyped here — `NewDiscoveryModal`'s `seedTitle`, same
   *  shape. Empty for every other way of opening this modal. */
  seedTitle: string;
  onClose: () => void;
  onCreated: (thread: AskThread) => void;
}

export function NewAskThreadModal({
  projectId,
  machineId,
  projectName,
  seedTitle,
  onClose,
  onCreated,
}: NewAskThreadModalProps): React.ReactElement {
  const { agents } = useAgentCatalog();
  const [title, setTitle] = useState(seedTitle);
  const [agentKind, setAgentKind] = useState('');
  const [model, setModel] = useState('');
  const [effort, setEffort] = useState<EffortLevel>(DEFAULT_EFFORT);
  const [machine, setMachine] = useState(machineId);
  const [network, setNetwork] = useState(true);
  const [machines, setMachines] = useState<Machine[]>([]);
  const [models, setModels] = useState<ConfigOptionValue[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The catalog arrives after the first render, so the initial agent is
  // picked here rather than in `useState` — and only while none is chosen, so
  // a later catalog refresh cannot overwrite the user's pick.
  useEffect(() => {
    if (agentKind || agents.length === 0) return;
    setAgentKind(agents[0].kind);
  }, [agents, agentKind]);

  useEffect(() => {
    let cancelled = false;
    listMachines()
      .then((list) => {
        if (!cancelled) setMachines(list ?? []);
      })
      .catch(() => {
        if (!cancelled) setMachines([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const machineOptions = useMemo(
    () => interviewerMachineOptions(machines, machineId),
    [machines, machineId],
  );

  const effortLevels = useMemo(
    () => effortLevelsFor(agents, agentKind),
    [agents, agentKind],
  );
  const effortSupported = effortLevels.length > 0;

  // Picking an agent resets the model to that agent's first entry: a model
  // list is per-harness, so carrying the previous pick across would send one
  // harness another's model. The probe runs against the chosen host, which
  // is the one that will answer.
  useEffect(() => {
    if (!agentKind) return;
    let cancelled = false;
    setModelsLoading(true);
    setModel('');
    getAgentModels(machine, agentKind)
      .then((list) => {
        if (cancelled) return;
        const values = list ?? [];
        setModels(values);
        setModel(values[0]?.value ?? '');
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
  }, [agentKind, machine]);

  // An agent with no per-invocation effort control gets none sent, rather
  // than one silently dropped on the floor.
  useEffect(() => {
    if (effortSupported && !effortLevels.includes(effort)) setEffort(effortLevels[0]);
  }, [effortLevels, effortSupported, effort]);

  const name = nameFieldState(title);
  const canStart =
    title.trim().length > 0 && !name.overLimit && agentKind !== '' && !submitting;

  const start = async () => {
    if (!canStart) return;
    setSubmitting(true);
    setError(null);
    try {
      const thread = await createAskThread({
        projectId,
        title: title.trim(),
        agentKind,
        model: model || null,
        effort: effortSupported ? effort : null,
        machineId: machine,
        network,
      });
      onCreated(thread);
    } catch (err) {
      setError(formatError(err));
      setSubmitting(false);
    }
  };

  return (
    <Modal onClose={onClose} className="w-full max-w-[620px] px-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="New ask thread"
        className="glass-panel flex max-h-[85vh] flex-col overflow-hidden"
      >
        <div className="shrink-0 border-b border-white/5 px-5 py-[18px]">
          <p className="mb-1.5 font-heading text-[11px] font-semibold uppercase tracking-[0.15em] text-cyan-400">
            {projectName || 'this project'}
          </p>
          <h2 className="font-heading text-xl font-bold text-white">New ask thread</h2>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-[18px] overflow-y-auto p-5">
          <div>
            <FieldLabel htmlFor="ask-thread-title">Name this thread</FieldLabel>
            <input
              id="ask-thread-title"
              type="text"
              value={title}
              maxLength={TITLE_MAX_CHARS}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Ask about the auth flow"
              className="input-field"
              aria-describedby="ask-thread-title-hint"
            />
            <div className="mt-2 flex items-start justify-between gap-3">
              <p id="ask-thread-title-hint" className="text-[11px] text-slate-500">
                A label for your thread list. Say what you want answered in the first message.
              </p>
              {name.showCounter && (
                <span
                  data-testid="ask-thread-name-remaining"
                  className={`shrink-0 font-mono text-[11px] ${
                    name.overLimit ? 'text-ruby-300' : 'text-slate-500'
                  }`}
                >
                  {name.remaining}
                </span>
              )}
            </div>
          </div>

          <div>
            <FieldLabel>Agent</FieldLabel>
            <div role="radiogroup" aria-label="Agent" className="flex flex-wrap gap-2">
              {agents.map((agent) => (
                <OptionPill
                  key={agent.kind}
                  selected={agent.kind === agentKind}
                  onSelect={() => setAgentKind(agent.kind)}
                >
                  {agent.kind}
                </OptionPill>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
            <div>
              <FieldLabel>Model</FieldLabel>
              {modelsLoading ? (
                <p className="text-[11px] text-slate-500">Probing models…</p>
              ) : models.length === 0 ? (
                <p className="text-[11px] text-slate-500">
                  This agent lists no models here. It will run on its own default.
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
              <div role="radiogroup" aria-label="Effort" className="flex flex-wrap gap-2">
                {(effortSupported ? effortLevels : DIMMED_EFFORTS).map((level) => (
                  <OptionPill
                    key={level}
                    selected={effortSupported && level === effort}
                    unsupported={!effortSupported}
                    onSelect={() => setEffort(level)}
                  >
                    {EFFORT_LABELS[level]}
                  </OptionPill>
                ))}
              </div>
            </div>
          </div>

          {!effortSupported && agentKind && <EffortUnsupportedNote agentKind={agentKind} />}

          <div>
            <FieldLabel htmlFor="ask-thread-machine">Machine</FieldLabel>
            <select
              id="ask-thread-machine"
              value={machine}
              onChange={(e) => setMachine(e.target.value)}
              className="input-field cursor-pointer appearance-none bg-[var(--bg-app)]"
            >
              {machineOptions.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.label}
                </option>
              ))}
            </select>
            <p className="mt-2 text-[11px] text-slate-500">
              Starts on this project's own host, where its repository was cloned. Move it and
              the thread runs there instead.
            </p>
          </div>

          <div>
            <FieldLabel>Web access</FieldLabel>
            <div className="flex items-start gap-3.5 rounded-xl border border-emerald-500/25 bg-emerald-500/[0.06] p-3.5">
              <button
                type="button"
                role="switch"
                aria-checked={network}
                aria-label="Web access"
                data-testid="ask-new-thread-network-toggle"
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
                <p className="mt-1 text-[11.5px] leading-relaxed text-slate-500">
                  The posture the thread opens with. Its first turn runs on whatever this says,
                  so a thread that must never reach the network is opened off, not switched off
                  afterwards.
                </p>
                {agentKind === 'hermes' && <NetworkUnenforcedNote />}
              </div>
            </div>
          </div>

          <div
            data-testid="ask-new-thread-capability"
            className="rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2.5 text-[11px] leading-relaxed"
          >
            <p className="text-slate-500">
              This thread gets its own worktree, created on the first turn that needs the repo
              and reclaimed while idle.
            </p>
            <p className="mt-1.5 text-slate-400">
              {network
                ? 'It reads files, runs commands, and reaches the network there.'
                : 'It reads files and runs commands there.'}{' '}
              It writes nothing — no branch, no commit.
            </p>
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
            onClick={start}
            disabled={!canStart}
            className="btn-primary text-[13px] disabled:cursor-not-allowed disabled:opacity-40"
          >
            Start thread
          </button>
        </div>
      </div>
    </Modal>
  );
}

/**
 * AGENTS.md §2's "declare the capability unsupported and degrade honestly",
 * made visible. Hermes has its own sentence because the reason is specific:
 * the setting exists, it simply lives in a file Demeteo will not write.
 */
function EffortUnsupportedNote({ agentKind }: { agentKind: string }): React.ReactElement {
  return (
    <p className="rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2.5 text-[11px] leading-relaxed text-amber-200/90">
      {agentKind === 'hermes'
        ? 'Hermes exposes reasoning effort only through its own config file, which Demeteo does not write. Effort is unavailable for this agent — it will run at whatever that file already says.'
        : `${agentKind} exposes no per-invocation reasoning effort, so Demeteo has nothing to set. Effort is unavailable for this agent — it will run at whatever it is already configured to do.`}
    </p>
  );
}

export default NewAskThreadModal;
