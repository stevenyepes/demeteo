import React from 'react';
import { Check } from 'lucide-react';

import { changeLabel, dependencyChip } from '../../lib/decomposeReview';
import type { ProposedChange } from '../../types';
import { Chip } from '../ui/Chip';

interface DecomposeChangeCardProps {
  change: ProposedChange;
  accepted: boolean;
  /** This checkbox is one the backend named when it refused the subset. */
  refused: boolean;
  disabled: boolean;
  onToggle: () => void;
  /** `seq` for a stored ticket an edge points at; `null` for one this pass is
   *  proposing, which has no number yet. */
  seqOf: (id: string) => number | null;
}

/**
 * One reviewable change, and one checkbox (`DISCOVERY_UI_SPEC.md` §4.4–§4.7).
 *
 * **The whole card is the click target**, not just the box: the box is 18 px
 * and the thing being decided about is a paragraph.
 *
 * The ruby state is not "this change is bad" — it is the backend having
 * refused the *combination* currently checked. A subset of a valid proposal is
 * not itself valid, and the refusal names the tickets it is about, so it is
 * drawn on them rather than as a sentence at the bottom of a modal with
 * nothing to point at.
 */
export function DecomposeChangeCard({
  change,
  accepted,
  refused,
  disabled,
  onToggle,
  seqOf,
}: DecomposeChangeCardProps): React.ReactElement {
  const dependency = change.kind === 'added' ? dependencyChip(change, seqOf) : null;

  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={accepted}
      data-testid="decompose-change"
      data-change-id={change.id}
      disabled={disabled}
      onClick={onToggle}
      className={`flex w-full items-start gap-3.5 rounded-xl border px-4 py-3.5 text-left transition ${
        refused
          ? 'border-ruby-500/40 bg-ruby-500/5'
          : 'border-white/5 bg-white/[0.02] hover:border-white/10 hover:bg-white/[0.04]'
      } ${accepted ? '' : 'opacity-45'} disabled:cursor-not-allowed`}
    >
      <span
        aria-hidden="true"
        className={`mt-0.5 flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-[5px] border ${
          accepted ? 'border-violet-500 bg-violet-500 text-white' : 'border-white/15 text-transparent'
        }`}
      >
        <Check className="h-3 w-3" strokeWidth={3} />
      </span>

      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2.5">
          <span className="shrink-0 font-mono text-[10px] text-slate-500">
            {changeLabel(change)}
          </span>
          <span className="min-w-0 flex-1 text-sm font-medium text-slate-100">{change.title}</span>
        </span>

        {change.fields.length > 0 && (
          <span className="mt-2.5 flex flex-col gap-1.5 rounded-lg border border-white/[0.04] bg-[#050608]/70 px-3 py-2.5">
            {change.fields.map((field) => (
              <span key={field.field} className="flex items-baseline gap-2.5">
                <span className="w-[92px] shrink-0 font-mono text-[10px] font-bold uppercase tracking-[0.08em] text-slate-500">
                  {field.field}
                </span>
                <span className="min-w-0 flex-1 font-mono text-[11px] leading-relaxed">
                  <span className="block break-words text-ruby-400 line-through decoration-ruby-400/50">
                    {field.was || '—'}
                  </span>
                  <span className="block break-words text-emerald-400">{field.now || '—'}</span>
                </span>
              </span>
            ))}
          </span>
        )}

        {change.why && (
          <span className="mt-2 block text-xs leading-relaxed text-slate-400">{change.why}</span>
        )}

        {dependency && (
          <span className="mt-2 flex flex-wrap items-center gap-1.5">
            {change.workflow_name && (
              <Chip size="sm" tone="violet" maxWidth="12rem">
                {change.workflow_name}
              </Chip>
            )}
            {change.agent_kind && (
              <Chip size="sm" tone="cyan">
                {change.agent_kind}
              </Chip>
            )}
            <Chip size="sm" tone={dependency.tone} maxWidth="16rem">
              {dependency.label}
            </Chip>
          </span>
        )}
      </span>
    </button>
  );
}

export default DecomposeChangeCard;
