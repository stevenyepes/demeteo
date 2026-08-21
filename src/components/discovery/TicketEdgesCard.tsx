import React, { useMemo, useState } from 'react';
import { Plus, X } from 'lucide-react';

import type { RunStatusTone } from '../../lib/runStatus';
import { edgeOptions } from '../../lib/ticketEditor';
import { prerequisiteRows, type TicketIndex } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { Chip } from '../ui/Chip';
import { FieldLabel } from '../ui/FieldLabel';

const DOT: Record<RunStatusTone, string> = {
  emerald: 'bg-emerald-400',
  cyan: 'bg-cyan-400',
  violet: 'bg-violet-400',
  amber: 'bg-amber-400',
  ruby: 'bg-ruby-400',
  slate: 'bg-slate-400',
};

interface TicketEdgesCardProps {
  view: TicketView;
  /** The edges as the form holds them, which is not what the row holds until
   *  the save lands. */
  edges: string[];
  onChange: (next: string[]) => void;
  /** Every ticket in this Discovery, for the picker and for each row's own
   *  standing. */
  siblings: readonly TicketView[];
  index: TicketIndex;
  disabled: boolean;
}

/**
 * Card 4 — `Blocked by` (`DISCOVERY_UI_SPEC.md` §5.7).
 *
 * The rows are `prerequisiteRows`' own, so an edge reads here exactly as it
 * reads in the inspector: one bucket, one vocabulary. What the editor adds is
 * a remove button and the `·` between the id and the title.
 */
export function TicketEdgesCard({
  view,
  edges,
  onChange,
  siblings,
  index,
  disabled,
}: TicketEdgesCardProps): React.ReactElement {
  const [adding, setAdding] = useState(false);

  // Against the *draft* edges rather than the stored ones, so removing a row
  // takes effect on screen before the save. The standing of each row is still
  // the board's — nothing here recomputes a lane.
  const rows = useMemo(
    () => prerequisiteRows({ ...view, ticket: { ...view.ticket, blocked_by: edges } }, index),
    [view, edges, index],
  );
  const options = useMemo(
    () => edgeOptions(view.ticket, siblings, edges),
    [view.ticket, siblings, edges],
  );

  return (
    <div className="nested-card flex flex-col gap-3.5 px-4 py-3.5">
      <FieldLabel className="mb-0">Blocked by</FieldLabel>

      {rows.length === 0 ? (
        <p className="m-0 text-[11px] text-slate-500">None. Nothing in this discovery gates it.</p>
      ) : (
        <div className="flex flex-col gap-2">
          {rows.map((row) => (
            <div
              key={row.id}
              data-testid="ticket-edge"
              className="flex items-start gap-2.5 rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2.5"
            >
              <span
                aria-hidden="true"
                className={`mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full ${DOT[row.tone]}`}
              />
              <div className="min-w-0 flex-1">
                <p className="m-0 text-xs text-slate-200">
                  {row.label} · {row.title}
                </p>
                {row.note && (
                  <p className="m-0 mt-0.5 font-mono text-[10px] text-slate-500">{row.note}</p>
                )}
              </div>
              <Chip size="sm" tone={row.tone}>
                {row.state}
              </Chip>
              <button
                type="button"
                title="Remove this edge"
                aria-label="Remove this edge"
                disabled={disabled}
                onClick={() => onChange(edges.filter((id) => id !== row.id))}
                className="shrink-0 rounded-md p-1 text-slate-600 transition hover:bg-ruby-500/[0.08] hover:text-ruby-400 disabled:cursor-not-allowed disabled:opacity-40"
              >
                <X className="h-[15px] w-[15px]" aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
      )}

      {adding ? (
        <select
          aria-label="Add a prerequisite"
          value=""
          disabled={disabled}
          onChange={(event) => {
            if (event.target.value) onChange([...edges, event.target.value]);
            setAdding(false);
          }}
          className="input-field cursor-pointer appearance-none bg-[var(--bg-app)] text-[13px]"
        >
          <option value="">Pick a ticket…</option>
          {options.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label} · {option.title}
            </option>
          ))}
        </select>
      ) : (
        <button
          type="button"
          disabled={disabled || options.length === 0}
          onClick={() => setAdding(true)}
          className="inline-flex items-center gap-1.5 self-start rounded-md border border-dashed border-white/10 px-3 py-[7px] text-xs text-slate-400 transition hover:border-violet-500/35 hover:bg-violet-500/5 hover:text-violet-300 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Plus className="h-[13px] w-[13px]" aria-hidden="true" />
          Add an edge
        </button>
      )}

      <p className="m-0 text-[11px] leading-relaxed text-slate-500">
        Edges point only at tickets in this discovery. Anything outside it belongs in the
        description, sequenced by hand.
      </p>
    </div>
  );
}

export default TicketEdgesCard;
