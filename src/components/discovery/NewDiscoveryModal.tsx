import React, { useEffect, useMemo, useRef, useState } from 'react';

import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { getAgentModels, modelSupportsImages } from '../../lib/agentModels';
import { stagedAttachmentInputs } from '../../lib/attachments';
import { createDiscovery, setDiscoveryBase } from '../../lib/discovery';
import { resolveBaseBranchInput } from '../../lib/discoveryBase';
import { DEFAULT_EFFORT, EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import { listMachines } from '../../lib/machines';
import {
  interviewerMachineOptions,
  nameFieldState,
  noVisionNote,
  suggestedBranchName,
  TITLE_MAX_CHARS,
} from '../../lib/newDiscovery';
import { getProposedStrategy, getRepositoriesForProject } from '../../lib/project';
import { listTerminalBranches } from '../../lib/terminal';
import type { ConfigOptionValue, Discovery, Machine, TerminalBranchOption } from '../../types';
import { AttachmentDropzone, type LaunchStageEntry } from '../AttachmentDropzone';
import { FieldLabel } from '../ui/FieldLabel';
import { Modal } from '../ui/Modal';
import { OptionPill } from './OptionPill';

/** What the dimmed group shows when the harness has no effort control at all.
 *  The pills stay on screen because the note below them is about *these*
 *  three, and a note pointing at nothing explains nothing. */
const DIMMED_EFFORTS: readonly EffortLevel[] = ['low', 'medium', 'high'];

interface NewDiscoveryModalProps {
  projectId: string;
  /** The project's own host, which is where its repository was cloned. The
   *  picker starts there and the user may move it: §4.5 makes the host part of
   *  the interviewer choice, so a value they give is never overridden. */
  machineId: string;
  /** Whatever was typed into the hero card, carried in as the starting name.
   *  It arrives past [`TITLE_MAX_CHARS`] only when something set it without a
   *  keystroke, which `maxLength` cannot stop — the counter is what says so. */
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
  const [machine, setMachine] = useState(machineId);
  const [machines, setMachines] = useState<Machine[]>([]);
  const [models, setModels] = useState<ConfigOptionValue[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [attachments, setAttachments] = useState<LaunchStageEntry[]>([]);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [baseMode, setBaseMode] = useState<'default' | 'named'>('default');
  const [baseName, setBaseName] = useState('');
  const [baseNameEdited, setBaseNameEdited] = useState(false);
  const [createdDiscovery, setCreatedDiscovery] = useState<Discovery | null>(null);
  const [branchPrefix, setBranchPrefix] = useState('');
  const [defaultBranchName, setDefaultBranchName] = useState<string | null>(null);
  const [branchOptions, setBranchOptions] = useState<TerminalBranchOption[]>([]);

  // The catalog arrives after the first render, so the initial interviewer is
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

  // Live host round trip the modal otherwise never makes — deferred until the
  // named-branch choice is actually reached, mirroring `OriginPicker`'s
  // load-on-open rather than this file's own eager `listMachines()`.
  const branchDataRequested = useRef(false);
  useEffect(() => {
    if (baseMode !== 'named' || branchDataRequested.current) return;
    branchDataRequested.current = true;
    let cancelled = false;
    (async () => {
      try {
        const repos = await getRepositoriesForProject(projectId);
        const repositoryId = repos[0]?.id;
        if (!repositoryId) throw new Error('no repository');
        const [strategy, branches] = await Promise.all([
          getProposedStrategy(projectId),
          listTerminalBranches(projectId, repositoryId),
        ]);
        if (cancelled) return;
        setBranchPrefix(strategy?.worktree_strategy.branch_prefix ?? '');
        setDefaultBranchName(branches.defaultBranch);
        setBranchOptions(branches.branches);
      } catch {
        if (cancelled) return;
        setBranchPrefix('');
        setDefaultBranchName(null);
        setBranchOptions([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [baseMode, projectId]);

  // Same "seed until touched" shape as the `agentKind` effect above: a fresh
  // suggestion arrives whenever the inputs it derives from change, but only
  // until the user's own `onChange` marks the field as theirs.
  useEffect(() => {
    if (baseMode !== 'named' || baseNameEdited) return;
    setBaseName(suggestedBranchName(title, branchPrefix));
  }, [baseMode, baseNameEdited, title, branchPrefix]);

  const effortLevels = useMemo(
    () => effortLevelsFor(agents, agentKind),
    [agents, agentKind],
  );
  const effortSupported = effortLevels.length > 0;

  // Picking an interviewer resets the model to that agent's first entry
  // (`DISCOVERY_UI_SPEC.md` §2.4): a model list is per-harness, so carrying
  // the previous pick across would send one harness another's model. The
  // probe runs against the chosen host, which is the one that will answer.
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

  // An agent with no per-invocation effort control gets none sent, rather than
  // one silently dropped on the floor.
  useEffect(() => {
    if (effortSupported && !effortLevels.includes(effort)) setEffort(effortLevels[0]);
  }, [effortLevels, effortSupported, effort]);

  // Probe-aware rather than name-only: the model list is already in hand, so
  // the backend's own `supports_images` answers, and the name heuristic is
  // reached only for a model it does not carry.
  const noVision = noVisionNote({
    model,
    readsImages: modelSupportsImages(models, agentKind, model),
    attachments,
  });

  const name = nameFieldState(title);
  const canStart =
    title.trim().length > 0 &&
    !name.overLimit &&
    agentKind !== '' &&
    !submitting &&
    (baseMode === 'default' || baseName.trim().length > 0);

  const start = async () => {
    if (!canStart) return;
    setSubmitting(true);
    setError(null);
    try {
      // `createdDiscovery` doubles as the retry guard: a prior press that
      // created the Discovery but failed on `setDiscoveryBase` must not
      // create a second row on the next press.
      const discovery =
        createdDiscovery ??
        (await createDiscovery({
          projectId,
          title: title.trim(),
          agentKind,
          model: model || null,
          effort: effortSupported ? effort : null,
          machineId: machine,
          stagedAttachments: await stagedAttachmentInputs(attachments),
        }));
      if (!createdDiscovery) setCreatedDiscovery(discovery);
      const branch = resolveBaseBranchInput(baseMode === 'default', baseName);
      if (branch !== null) await setDiscoveryBase(discovery.id, branch);
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
            <FieldLabel htmlFor="discovery-seed">Name this discovery</FieldLabel>
            <input
              id="discovery-seed"
              type="text"
              value={title}
              maxLength={TITLE_MAX_CHARS}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Ask-the-repo chat"
              className="input-field"
              aria-describedby="discovery-seed-hint"
            />
            <div className="mt-2 flex items-start justify-between gap-3">
              <p id="discovery-seed-hint" className="text-[11px] text-slate-500">
                A label for your discovery list — the interviewer never reads it. Say the idea
                itself in the first message; that is what it is asked about.
              </p>
              {name.showCounter && (
                <span
                  data-testid="discovery-name-remaining"
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
            <FieldLabel htmlFor="discovery-machine">Machine</FieldLabel>
            <select
              id="discovery-machine"
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
              the interview runs there instead.
            </p>
          </div>

          <div>
            <FieldLabel>Base branch</FieldLabel>
            <div role="radiogroup" aria-label="Base branch" className="flex flex-wrap gap-2">
              <OptionPill selected={baseMode === 'default'} onSelect={() => setBaseMode('default')}>
                Project default
              </OptionPill>
              <OptionPill selected={baseMode === 'named'} onSelect={() => setBaseMode('named')}>
                Named branch
              </OptionPill>
            </div>
            {baseMode === 'named' && (
              <div className="mt-2.5">
                <input
                  id="discovery-base-name"
                  type="text"
                  value={baseName}
                  onChange={(e) => {
                    setBaseNameEdited(true);
                    setBaseName(e.target.value);
                  }}
                  list="discovery-base-branch-options"
                  placeholder="demeteo/features/my-integration-branch"
                  className="input-field"
                  aria-label="Base branch name"
                />
                <datalist id="discovery-base-branch-options">
                  {branchOptions.map((option) => (
                    <option key={option.name} value={option.name} />
                  ))}
                </datalist>
              </div>
            )}
            <p className="mt-2 text-[11px] text-slate-500">
              {baseMode === 'default'
                ? defaultBranchName
                  ? `Tickets cut their branches from ${defaultBranchName}.`
                  : "Tickets cut their branches from the project's default branch."
                : 'Tickets cut their branches from this one and merge back into it.'}
            </p>
          </div>

          <div>
            <FieldLabel>Attachments</FieldLabel>
            <AttachmentDropzone
              mode="launch"
              label="Attach"
              stageEntries={attachments}
              onChangeStage={setAttachments}
              onError={setAttachmentError}
              maxChips={6}
            />
            {attachmentError && (
              <p role="alert" className="mt-2 font-mono text-[11px] text-ruby-200">
                {attachmentError}
              </p>
            )}
            {noVision && (
              <p
                data-testid="discovery-no-vision"
                className="mt-2 rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2.5 text-[11px] leading-relaxed text-amber-200/90"
              >
                {noVision.model} cannot read images.{' '}
                <span className="font-mono">{noVision.filenames.join(', ')}</span> will be
                attached and ignored.
              </p>
            )}
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
              {createdDiscovery !== null
                ? `Discovery created, but its base branch could not be set: ${error}`
                : error}
            </p>
          )}
        </div>

        <div className="flex shrink-0 justify-end gap-2.5 border-t border-white/5 bg-[#0d0f14]/90 px-5 py-4">
          <button
            type="button"
            onClick={() => {
              // A Discovery created before a failed `setDiscoveryBase` already
              // exists server-side; closing without `onCreated` would leave it
              // outside DiscoverySection's one-shot fetch until an unrelated
              // refetch surfaces it. The label flips to "Close" for the same
              // reason: "Cancel" would promise a clean walk-away this cannot give.
              if (createdDiscovery !== null) onCreated(createdDiscovery);
              else onClose();
            }}
            className="btn-secondary text-[13px]"
          >
            {createdDiscovery !== null ? 'Close' : 'Cancel'}
          </button>
          <button
            type="button"
            onClick={start}
            disabled={!canStart}
            className="btn-primary text-[13px] disabled:cursor-not-allowed disabled:opacity-40"
          >
            {createdDiscovery !== null ? 'Retry base branch' : 'Start discovery'}
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
