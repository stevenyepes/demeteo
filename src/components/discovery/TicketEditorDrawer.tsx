import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Lock } from 'lucide-react';

import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { getAgentModels, modelSupportsImages } from '../../lib/agentModels';
import { getTicketBriefing, updateTicket } from '../../lib/discovery';
import { ticketLabel } from '../../lib/discoveryProgress';
import { EFFORT_LABELS, type EffortLevel, isEffortLevel } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import {
  draftOf,
  editOf,
  isDirty,
  isTicketLocked,
  type TicketDraft,
} from '../../lib/ticketEditor';
import { stateLabel, ticketTone, type TicketIndex } from '../../lib/ticketPresentation';
import type {
  ConfigOptionValue,
  DiscoveryBoard,
  TicketView,
  WorkflowWithSteps,
} from '../../types';
import { Chip } from '../ui/Chip';
import { FieldLabel } from '../ui/FieldLabel';
import { ColumnSubHeader } from './ColumnSubHeader';
import { LabelledSelect } from './LabelledSelect';
import { TicketAttachmentsCard } from './TicketAttachmentsCard';
import { TicketEdgesCard } from './TicketEdgesCard';
import { TicketFieldList } from './TicketFieldList';
import { TicketForceStart } from './TicketForceStart';

interface TicketEditorDrawerProps {
  view: TicketView;
  index: TicketIndex;
  siblings: readonly TicketView[];
  workflows: readonly WorkflowWithSteps[];
  /** Where the models are probed — the Discovery's own host, which is the one
   *  that will answer. */
  machineId: string;
  busy: boolean;
  onClose: () => void;
  /** `ticket_update` returns the whole board, because an edited edge moves the
   *  standing of everything under it. */
  onSaved: (board: DiscoveryBoard) => void;
  onRefresh: () => void;
  onStart: () => void;
  onForceStart: (reason: string) => void;
  onDrop: (reason: string) => void;
}

/**
 * The full editor for one Ticket (`DISCOVERY_UI_SPEC.md` §5, PRD §5.4) — a
 * wider right-hand drawer in place of the 360 px inspector, not a modal.
 *
 * **A locked ticket is shown as locked, not allowed to fail on save.** §5.4
 * locks a Ticket the moment it has a Feature; its run is already working
 * against the plan as it stands, so every control below goes read-only and the
 * save button is not drawn at all. Letting the form take the edit and the
 * backend refuse it would be the same rule enforced one round trip later, and
 * with the user's typing thrown away.
 *
 * **The save is the whole ticket.** Every key of `TicketEdit` is required on
 * the wire — serde reads an absent key and an explicit `null` the same way —
 * so there is no patch shape here and no per-field dirty tracking to build one
 * out of.
 */
export function TicketEditorDrawer({
  view,
  index,
  siblings,
  workflows,
  machineId,
  busy,
  onClose,
  onSaved,
  onRefresh,
  onStart,
  onForceStart,
  onDrop,
}: TicketEditorDrawerProps): React.ReactElement {
  const { ticket } = view;
  const locked = isTicketLocked(ticket);

  const [draft, setDraft] = useState<TicketDraft>(() => draftOf(ticket));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [briefing, setBriefing] = useState<string | null>(null);
  const [models, setModels] = useState<ConfigOptionValue[]>([]);

  const { agents } = useAgentCatalog();
  const agentKinds = useMemo(() => agents.map((agent) => agent.kind), [agents]);
  const efforts = useMemo(
    () => effortLevelsFor(agents, draft.agentKind),
    [agents, draft.agentKind],
  );

  const ticketId = ticket.id;
  const updatedAt = ticket.updated_at;

  // Re-seeded whenever the stored row *moves* — a save, a force start, an
  // attachment — and never on the identity of the object carrying it. Every
  // board refresh hands back a fresh `ticket`, so depending on it would wipe
  // the draft mid-edit on any unrelated change elsewhere in the plan.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `updated_at` is the dependency; `ticket` is a fresh object on every refresh.
  useEffect(() => {
    setDraft(draftOf(ticket));
    setError(null);
  }, [ticketId, updatedAt]);

  const readBriefing = useCallback(() => {
    let cancelled = false;
    getTicketBriefing(ticketId)
      .then((text) => {
        if (!cancelled) setBriefing(text);
      })
      .catch((cause) => {
        if (!cancelled) setBriefing(formatError(cause));
      });
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  // Composed by the backend from the stored row, so it is re-read whenever
  // that row moves: it is what the agent *will* be told, not a preview of an
  // unsaved form. §5.8's force-start paragraph appears through exactly this.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `updated_at` is what makes the briefing stale; it is a dependency of the fetch, not of the callback.
  useEffect(() => readBriefing(), [readBriefing, updatedAt]);

  useEffect(() => {
    if (!draft.agentKind) {
      setModels([]);
      return;
    }
    let cancelled = false;
    getAgentModels(machineId, draft.agentKind)
      .then((list) => {
        if (!cancelled) setModels(list ?? []);
      })
      .catch(() => {
        if (!cancelled) setModels([]);
      });
    return () => {
      cancelled = true;
    };
  }, [machineId, draft.agentKind]);

  function set<K extends keyof TicketDraft>(key: K, value: TicketDraft[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      onSaved(await updateTicket(ticketId, editOf(draft)));
    } catch (cause) {
      setError(formatError(cause));
    } finally {
      setSaving(false);
    }
  }

  const dirty = isDirty(draft, ticket);
  const disabled = locked || saving || busy;

  return (
    <div
      data-testid="ticket-editor"
      className="flex w-[760px] min-h-0 shrink-0 flex-col overflow-y-auto border-l border-white/5 bg-[#0d0f14]"
    >
      <ColumnSubHeader title={ticketLabel(ticket.seq)} sticky>
        <Chip size="sm" tone={ticketTone(view, index)} dot>
          {stateLabel(view)}
        </Chip>
        <Chip size="sm" tone="slate">
          {locked ? 'Locked' : 'Unstarted'}
        </Chip>
        <button type="button" onClick={onClose} className="btn-secondary text-xs">
          {locked ? 'Close' : 'Discard'}
        </button>
        {!locked && (
          <button
            type="button"
            data-testid="ticket-save"
            disabled={!dirty || disabled}
            onClick={() => void save()}
            className="btn-primary text-xs disabled:cursor-not-allowed disabled:opacity-35"
          >
            {saving ? 'Saving…' : 'Save ticket'}
          </button>
        )}
      </ColumnSubHeader>

      <div className="flex flex-col gap-3.5 p-5">
        {locked ? (
          <p
            data-testid="ticket-locked"
            className="m-0 flex items-start gap-2 rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2.5 text-[11px] leading-relaxed text-slate-400"
          >
            <Lock className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-500" aria-hidden="true" />
            This ticket has a feature, so it is no longer editable. Its run is working against the
            plan as it stands — add a follow-up ticket for the change instead.
          </p>
        ) : (
          <p className="m-0 text-[11px] leading-relaxed text-slate-500">
            Every field is yours while this ticket has no feature. Starting it locks the lot — a
            started ticket is never revised, removed or renumbered.
          </p>
        )}

        {error && (
          <p role="alert" className="m-0 font-mono text-[11px] text-ruby-200">
            {error}
          </p>
        )}

        <div className="nested-card flex flex-col gap-3.5 px-4 py-3.5">
          <FieldLabel className="mb-0">The work</FieldLabel>

          <div>
            <FieldLabel htmlFor="ticket-title">Title</FieldLabel>
            <input
              id="ticket-title"
              type="text"
              value={draft.title}
              disabled={disabled}
              onChange={(event) => set('title', event.target.value)}
              className="input-field text-[13px]"
            />
          </div>

          <div>
            <FieldLabel htmlFor="ticket-description">Description</FieldLabel>
            <textarea
              id="ticket-description"
              rows={4}
              value={draft.description}
              disabled={disabled}
              onChange={(event) => set('description', event.target.value)}
              className="input-field text-[13px] leading-relaxed"
            />
          </div>

          <TicketFieldList
            label="Acceptance"
            values={draft.acceptance}
            onChange={(next) => set('acceptance', next)}
            numbered
            addLabel="Add criterion"
            removeTitle="Remove this criterion"
            disabled={disabled}
          />

          <div className="grid grid-cols-2 gap-4">
            <TicketFieldList
              label="Files"
              values={draft.files}
              onChange={(next) => set('files', next)}
              mono
              addLabel="Add path"
              removeTitle="Remove this path"
              disabled={disabled}
            />
            <div>
              <FieldLabel htmlFor="ticket-test-command">Test command</FieldLabel>
              <input
                id="ticket-test-command"
                type="text"
                value={draft.testCommand}
                disabled={disabled}
                onChange={(event) => set('testCommand', event.target.value)}
                className="input-field font-mono text-xs"
              />
              <p className="m-0 mt-2 text-[11px] leading-relaxed text-slate-500">
                Inside a run, the full <span className="font-mono">checks</span> judges commits this
                ticket never wrote.
              </p>
            </div>
          </div>
        </div>

        <TicketAttachmentsCard
          ticketId={ticketId}
          attachments={ticket.attachments}
          model={draft.model}
          readsImages={modelSupportsImages(models, draft.agentKind, draft.model)}
          onChanged={onRefresh}
          disabled={locked}
        />

        <div className="nested-card flex flex-col gap-3.5 px-4 py-3.5">
          <div className="flex items-baseline justify-between gap-3">
            <FieldLabel className="mb-0">Execution</FieldLabel>
            <span className="text-[11px] text-slate-500">
              Per ticket — a plan whose parts want different agents can say so.
            </span>
          </div>
          <div className="grid grid-cols-2 gap-2.5">
            <LabelledSelect
              label="Workflow"
              value={draft.workflowId}
              disabled={disabled}
              onChange={(value) => set('workflowId', value)}
              options={workflows.map((workflow) => ({ value: workflow.id, label: workflow.name }))}
            />
            <LabelledSelect
              label="Agent"
              value={draft.agentKind}
              disabled={disabled}
              onChange={(value) => set('agentKind', value)}
              options={agentKinds.map((kind) => ({ value: kind, label: kind }))}
            />
            <LabelledSelect
              label="Model"
              value={draft.model}
              disabled={disabled}
              onChange={(value) => set('model', value)}
              options={models.map((option) => ({ value: option.value, label: option.value }))}
            />
            <LabelledSelect
              label="Effort"
              value={draft.effort}
              disabled={disabled || efforts.length === 0}
              onChange={(value) => set('effort', isEffortLevel(value) ? value : '')}
              options={efforts.map((level: EffortLevel) => ({
                value: level,
                label: `effort: ${EFFORT_LABELS[level].toLowerCase()}`,
              }))}
            />
          </div>
        </div>

        <TicketEdgesCard
          view={view}
          edges={draft.blockedBy}
          onChange={(next) => set('blockedBy', next)}
          siblings={siblings}
          index={index}
          disabled={disabled}
        />

        <div className="rounded-xl border border-white/5 bg-[#050608]/80 px-4 py-3.5">
          <FieldLabel>What its agent will be told</FieldLabel>
          <p className="m-0 whitespace-pre-wrap font-mono text-[11px] leading-loose text-slate-400">
            {briefing ?? '…'}
          </p>
        </div>

        <TicketForceStart
          view={view}
          index={index}
          busy={disabled}
          onStart={onStart}
          onForceStart={onForceStart}
          onDrop={onDrop}
        />
      </div>
    </div>
  );
}

export default TicketEditorDrawer;
