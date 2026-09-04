import React, { useEffect, useState } from 'react';

import { getTicketBriefing } from '../../lib/discovery';
import { ticketLabel } from '../../lib/discoveryProgress';
import { EFFORT_LABELS } from '../../lib/effortLevels';
import { formatError } from '../../lib/errors';
import type { RunStatusTone } from '../../lib/runStatus';
import {
  prerequisiteRows,
  primaryAction,
  showsForceStart,
  stateLabel,
  ticketTone,
  verdict,
  type TicketIndex,
} from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { Chip } from '../ui/Chip';
import { FieldLabel } from '../ui/FieldLabel';
import { AgentMarkdown } from './AgentMarkdown';
import { ColumnSubHeader } from './ColumnSubHeader';

const VERDICT_SURFACE: Record<RunStatusTone, string> = {
  emerald: 'border-emerald-500/20 bg-emerald-500/5',
  cyan: 'border-cyan-500/20 bg-cyan-500/5',
  violet: 'border-violet-500/20 bg-violet-500/5',
  amber: 'border-amber-500/20 bg-amber-500/5',
  ruby: 'border-ruby-500/20 bg-ruby-500/5',
  slate: 'border-white/5 bg-white/[0.02]',
};

const VERDICT_DOT: Record<RunStatusTone, string> = {
  emerald: 'bg-emerald-400',
  cyan: 'bg-cyan-400',
  violet: 'bg-violet-400',
  amber: 'bg-amber-400',
  ruby: 'bg-ruby-400',
  slate: 'bg-slate-400',
};

const VERDICT_TEXT: Record<RunStatusTone, string> = {
  emerald: 'text-emerald-400',
  cyan: 'text-cyan-400',
  violet: 'text-violet-400',
  amber: 'text-amber-400',
  ruby: 'text-ruby-400',
  slate: 'text-slate-400',
};

/** §5.9's rule: below this the reason is not one. */
const MIN_REASON = 8;

interface TicketInspectorProps {
  view: TicketView;
  index: TicketIndex;
  /** Resolved from `workflow_list`; the ticket stores only an id. */
  workflowName: string | null;
  onStart: () => void;
  onForceStart: (reason: string) => void;
  /** Open the full editor (§3.6.8). Offered on every ticket: a locked one is
   *  read there rather than refused here, which is where the reason it is
   *  locked can actually be shown. */
  onEdit: () => void;
  onOpenFeature: (featureId: string) => void;
  onClose: () => void;
  busy: boolean;
}

/**
 * The read surface beside the graph (`DISCOVERY_UI_SPEC.md` §3.6): what this
 * ticket is, why it stands where it stands, and the one thing you can do about
 * it from here.
 *
 * Every verdict on this panel is recomputed on the way past — nothing below is
 * read from a stored column, which is the property §6.3 spends a section
 * defending and §6.7 keeps the copy saying out loud.
 */
export function TicketInspector({
  view,
  index,
  workflowName,
  onStart,
  onForceStart,
  onEdit,
  onOpenFeature,
  onClose,
  busy,
}: TicketInspectorProps): React.ReactElement {
  const [briefing, setBriefing] = useState<string | null>(null);
  const [briefingError, setBriefingError] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const [reason, setReason] = useState('');

  const ticketId = view.ticket.id;

  useEffect(() => {
    let cancelled = false;
    setBriefing(null);
    setBriefingError(null);
    setAsking(false);
    setReason('');
    getTicketBriefing(ticketId)
      .then((text) => {
        if (!cancelled) setBriefing(text);
      })
      .catch((cause) => {
        if (!cancelled) setBriefingError(formatError(cause));
      });
    return () => {
      cancelled = true;
    };
  }, [ticketId]);

  const tone = ticketTone(view, index);
  const standing = verdict(view, index);
  const action = primaryAction(view, index);
  const prerequisites = prerequisiteRows(view, index);
  const blockedLabels = view.standing.blockers.map((blocker) => {
    const prerequisite = index.get(blocker.id);
    return prerequisite ? ticketLabel(prerequisite.ticket.seq) : 'an unknown ticket';
  });
  const forced = view.ticket.force_started_at !== null;

  return (
    <div className="flex w-[360px] min-h-0 shrink-0 flex-col overflow-y-auto border-l border-white/5 bg-[#0d0f14]/70">
      <ColumnSubHeader title={ticketLabel(view.ticket.seq)} sticky>
        <Chip size="sm" tone={tone}>
          {stateLabel(view)}
        </Chip>
        <button type="button" onClick={onClose} className="btn-secondary text-xs">
          Close
        </button>
      </ColumnSubHeader>

      <div className="flex flex-col gap-[18px] p-4">
        <div>
          <h2 className="m-0 mb-1.5 font-heading text-base font-semibold leading-snug text-white">
            {view.ticket.title}
          </h2>
          {view.ticket.description && (
            <AgentMarkdown text={view.ticket.description} size="dense" />
          )}
        </div>

        <div
          data-testid="ticket-verdict"
          className={`rounded-xl border px-3.5 py-3 ${VERDICT_SURFACE[tone]}`}
        >
          <div className="flex items-center gap-2">
            <span aria-hidden="true" className={`h-1.5 w-1.5 rounded-full ${VERDICT_DOT[tone]}`} />
            <span className={`font-heading text-[13px] font-semibold ${VERDICT_TEXT[tone]}`}>
              {standing.label}
            </span>
          </div>
          <p className="m-0 mt-1.5 text-[11px] leading-relaxed text-slate-400">{standing.why}</p>
        </div>

        <div>
          <FieldLabel>Prerequisites</FieldLabel>
          {prerequisites.length === 0 ? (
            <p className="m-0 text-[11px] text-slate-500">
              None. Nothing in this discovery gates it.
            </p>
          ) : (
            <div className="flex flex-col gap-2">
              {prerequisites.map((row) => (
                <div
                  key={row.id}
                  className="flex items-start gap-2.5 rounded-lg border border-white/5 bg-white/[0.02] px-2.5 py-2"
                >
                  <span
                    aria-hidden="true"
                    className={`mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full ${VERDICT_DOT[row.tone]}`}
                  />
                  <div className="min-w-0 flex-1">
                    <p className="m-0 text-xs text-slate-200">
                      {row.label} {row.title}
                    </p>
                    {row.note && (
                      <p className="m-0 mt-0.5 font-mono text-[10px] text-slate-500">{row.note}</p>
                    )}
                  </div>
                  <Chip size="sm" tone={row.tone}>
                    {row.state}
                  </Chip>
                </div>
              ))}
            </div>
          )}
        </div>

        <div>
          <FieldLabel>Execution</FieldLabel>
          <div className="flex flex-wrap items-center gap-1.5">
            {workflowName && (
              <Chip size="sm" tone="violet" maxWidth="12rem">
                {workflowName}
              </Chip>
            )}
            {view.ticket.agent_kind && (
              <Chip size="sm" tone="cyan">
                {view.ticket.agent_kind}
              </Chip>
            )}
            {view.ticket.model && (
              <Chip size="sm" tone="violet" maxWidth="10rem">
                {view.ticket.model}
              </Chip>
            )}
            {view.ticket.effort && (
              <Chip size="sm" tone="slate">
                effort {EFFORT_LABELS[view.ticket.effort]}
              </Chip>
            )}
          </div>
        </div>

        <div>
          <FieldLabel>Acceptance</FieldLabel>
          {view.ticket.state === 'dropped' || view.ticket.acceptance.length === 0 ? (
            <p className="m-0 text-xs text-slate-300">—</p>
          ) : (
            <ul className="m-0 list-disc pl-4 text-xs leading-loose text-slate-300">
              {view.ticket.acceptance.map((criterion) => (
                <li key={criterion}>{criterion}</li>
              ))}
            </ul>
          )}
        </div>

        {view.ticket.files.length > 0 && (
          <div>
            <FieldLabel>Files</FieldLabel>
            <div className="flex flex-col gap-1">
              {view.ticket.files.map((path) => (
                <span key={path} className="font-mono text-[11px] break-all text-slate-400">
                  {path}
                </span>
              ))}
            </div>
          </div>
        )}

        <div className="rounded-xl border border-white/5 bg-[#050608]/80 px-3.5 py-3">
          <FieldLabel>What its agent will be told</FieldLabel>
          <p className="m-0 whitespace-pre-wrap font-mono text-[11px] leading-loose text-slate-400">
            {briefingError ?? briefing ?? '…'}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            data-testid="ticket-primary-action"
            disabled={action.disabled || busy}
            onClick={() => {
              if (action.kind === 'start') onStart();
              if (action.kind === 'open' && view.feature) onOpenFeature(view.feature.id);
            }}
            className="btn-primary flex-1 disabled:cursor-not-allowed disabled:opacity-35"
          >
            {action.label}
          </button>
          <button
            type="button"
            data-testid="ticket-edit"
            onClick={onEdit}
            className="btn-secondary shrink-0"
          >
            Edit
          </button>
        </div>

        {forced && (
          <div className="rounded-xl border border-emerald-500/25 bg-emerald-500/5 px-3.5 py-3">
            <div className="flex items-center gap-2">
              <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
              <span className="font-heading text-[13px] font-semibold text-emerald-400">
                Force-started
              </span>
            </div>
            <p className="m-0 mt-2 font-mono text-[11px] leading-loose text-slate-400">
              {new Date(view.ticket.force_started_at ?? 0).toLocaleString()}
              <br />
              {`“${view.ticket.force_start_reason ?? ''}”`}
            </p>
            <p className="m-0 mt-2.5 text-[11px] text-slate-500">
              Kept on the ticket, and repeated to the agent above.
            </p>
          </div>
        )}

        {showsForceStart(view) && !forced && (
          <div className="border-t border-white/5 pt-4">
            <FieldLabel className="text-amber-400!">Start it anyway</FieldLabel>
            <p className="m-0 mb-3 text-xs leading-relaxed text-slate-400">
              Nothing has released this ticket yet. Force start bypasses every edge at once — and
              records why, for you and for the agent that reads its own prerequisite list.
            </p>

            {asking ? (
              <div className="rounded-xl border border-amber-500/25 bg-amber-500/5 px-3.5 py-3">
                <FieldLabel className="text-amber-400!">
                  Why are you bypassing {blockedLabels.join(', ')}?
                </FieldLabel>
                <textarea
                  rows={3}
                  value={reason}
                  onChange={(event) => setReason(event.target.value)}
                  placeholder={`This project has no forge remote — ${blockedLabels[0] ?? 'it'} merged out of band this morning.`}
                  aria-label="Reason for bypassing the prerequisites"
                  className="input-field"
                />
                <div className="mt-3 flex items-center gap-2.5">
                  <button
                    type="button"
                    data-testid="ticket-force-confirm"
                    disabled={reason.trim().length < MIN_REASON || busy}
                    onClick={() => onForceStart(reason.trim())}
                    className="rounded-md border border-amber-500/25 bg-amber-500/[0.08] px-4 py-2.5 text-amber-400 transition hover:bg-amber-500/[0.14] disabled:cursor-not-allowed disabled:opacity-35"
                  >
                    Force start {ticketLabel(view.ticket.seq)}
                  </button>
                  <button type="button" onClick={() => setAsking(false)} className="btn-secondary">
                    Cancel
                  </button>
                  <span className="ml-auto text-[11px] text-slate-500">
                    {reason.trim().length < MIN_REASON
                      ? 'A reason is required.'
                      : 'Recorded on the ticket.'}
                  </span>
                </div>
              </div>
            ) : (
              <button
                type="button"
                data-testid="ticket-force-start"
                onClick={() => setAsking(true)}
                className="w-full rounded-md border border-amber-500/25 bg-amber-500/[0.08] px-4 py-2.5 text-amber-400 transition hover:bg-amber-500/[0.14]"
              >
                Force start with a reason&hellip;
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default TicketInspector;
