import React, { useEffect, useMemo, useState } from 'react';

import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { getAgentModels } from '../../lib/agentModels';
import { createDiscovery } from '../../lib/discovery';
import { DEFAULT_EFFORT, EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import type { Discovery } from '../../types';
import { FieldLabel } from '../ui/FieldLabel';
import { Modal } from '../ui/Modal';
import { OptionPill } from './OptionPill';

/** What the dimmed group shows when the harness has no effort control at all.
 *  The pills stay on screen because the note below them is about *these*
 *  three, and a note pointing at nothing explains nothing. */
const DIMMED_EFFORTS: readonly EffortLevel[] = ['low', 'medium', 'high'];

interface NewDiscoveryModalProps {
  projectId: string;
  /** The host the interview runs on. Not a choice: the repository it reads
   *  exists on exactly one machine, so `discovery_create` takes the project's
   *  and ignores anything else. */
  machineId: string;
  /** Whatever was typed into the hero card, carried in as the seed. */
  seedTitle: string;
  onClose: () => void;
  onCreated: (discovery: Discovery) => void;
}

export function NewDiscoveryModal({
  projectId,
  machineId,
  seedTitle,
  onClose,
  onCreated,
}: NewDiscoveryModalProps): React.ReactElement {
  const { agents } = useAgentCatalog();
  const [title, setTitle] = useState(seedTitle);
  const [agentKind, setAgentKind] = useState('');
  const [model, setModel] = useState('');
  const [effort, setEffort] = useState<EffortLevel>(DEFAULT_EFFORT);
  const [models, setModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The catalog arrives after the first render, so the initial interviewer is
  // picked here rather than in `useState` — and only while none is chosen, so
  // a later catalog refresh cannot overwrite the user's pick.
  useEffect(() => {
    if (agentKind || agents.length === 0) return;
    setAgentKind(agents[0].kind);
  }, [agents, agentKind]);

  const effortLevels = useMemo(
    () => effortLevelsFor(agents, agentKind),
    [agents, agentKind],
  );
  const effortSupported = effortLevels.length > 0;

  // Picking an interviewer resets the model to that agent's first entry
  // (`DISCOVERY_UI_SPEC.md` §2.4): a model list is per-harness, so carrying
  // the previous pick across would send one harness another's model.
  useEffect(() => {
    if (!agentKind) return;
    let cancelled = false;
    setModelsLoading(true);
    setModel('');
    getAgentModels(machineId, agentKind)
      .then((list) => {
        if (cancelled) return;
        const values = (list ?? []).map((m) => m.value);
        setModels(values);
        setModel(values[0] ?? '');
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
  }, [agentKind, machineId]);

  // An agent with no per-invocation effort control gets none sent, rather than
  // one silently dropped on the floor.
  useEffect(() => {
    if (effortSupported && !effortLevels.includes(effort)) setEffort(effortLevels[0]);
  }, [effortLevels, effortSupported, effort]);

  const canStart = title.trim().length > 0 && agentKind !== '' && !submitting;

  const start = async () => {
    if (!canStart) return;
    setSubmitting(true);
    setError(null);
    try {
      const discovery = await createDiscovery({
        projectId,
        title: title.trim(),
        agentKind,
        model: model || null,
        effort: effortSupported ? effort : null,
      });
      onCreated(discovery);
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
        aria-label="New discovery"
        className="glass-panel flex max-h-[85vh] flex-col overflow-hidden"
      >
        <div className="shrink-0 border-b border-white/5 px-5 py-[18px]">
          <p className="mb-1.5 font-heading text-[11px] font-semibold uppercase tracking-[0.15em] text-cyan-400">
            demeteo
          </p>
          <h2 className="font-heading text-xl font-bold text-white">New discovery</h2>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-[18px] overflow-y-auto p-5">
          <div>
            <FieldLabel htmlFor="discovery-seed">What are you trying to work out?</FieldLabel>
            <textarea
              id="discovery-seed"
              rows={3}
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="A fuzzy idea, or work you already understand and want sharpened."
              className="input-field resize-none leading-relaxed"
            />
          </div>

          <div>
            <FieldLabel>Interviewer</FieldLabel>
            <div role="radiogroup" aria-label="Interviewer" className="flex flex-wrap gap-2">
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
            <p className="mt-2 text-[11px] text-slate-500">
              Chosen here, not inherited. Interviewing and implementing want different things
              from a model.
            </p>
          </div>

          <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2">
            <div>
              <FieldLabel>Model</FieldLabel>
              {modelsLoading ? (
                <p className="text-[11px] text-slate-500">Probing models…</p>
              ) : models.length === 0 ? (
                <p className="text-[11px] text-slate-500">
                  This interviewer lists no models here. It will run on its own default.
                </p>
              ) : (
                <div role="radiogroup" aria-label="Model" className="flex flex-wrap gap-2">
                  {models.map((value) => (
                    <OptionPill
                      key={value}
                      selected={value === model}
                      onSelect={() => setModel(value)}
                    >
                      {value}
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
            <FieldLabel htmlFor="discovery-machine">Machine</FieldLabel>
            <select
              id="discovery-machine"
              value={machineId}
              disabled
              className="input-field cursor-not-allowed appearance-none bg-[var(--bg-app)] opacity-60"
            >
              <option value={machineId}>{machineId}</option>
            </select>
            <p className="mt-2 text-[11px] text-slate-500">
              The repository this interview reads exists on one host, so it runs there.
            </p>
          </div>

          <div className="rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2.5 text-[11px] leading-relaxed">
            <p className="text-slate-500">
              This discovery gets its own worktree, created on the first turn that needs the
              repo and reclaimed while idle.
            </p>
            <p className="mt-1.5 text-slate-400">
              It reads files and runs commands there. It is given no write tools, and it leaves
              nothing behind — no branch, no committed spec. Whatever it writes rides to a
              feature as an attachment.
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
            Start discovery
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
        ? 'Hermes exposes reasoning effort only through its own config file, which Demeteo does not write. Effort is unavailable for this interviewer — it will run at whatever that file already says.'
        : `${agentKind} exposes no per-invocation reasoning effort, so Demeteo has nothing to set. Effort is unavailable for this interviewer — it will run at whatever it is already configured to do.`}
    </p>
  );
}

export default NewDiscoveryModal;
