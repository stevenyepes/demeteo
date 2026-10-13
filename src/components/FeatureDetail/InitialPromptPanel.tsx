import { useState } from 'react';
import { MessageSquareText } from 'lucide-react';

import { Disclosure } from '../ui/Disclosure';
import { PROSE_CH } from '../runLayout';
import { TONE_TEXT } from '../../lib/runStatus';

const SUMMARY_CHARS = 72;

function promptSummary(description: string): string {
  const firstLine = description.split('\n').find((line) => line.trim().length > 0)?.trim();
  if (!firstLine) return 'No prompt recorded';
  return firstLine.length > SUMMARY_CHARS ? `${firstLine.slice(0, SUMMARY_CHARS - 1)}…` : firstLine;
}

/**
 * The prompt the feature was launched with, verbatim — collapsed by default
 * (UI_REDESIGN_PLAN §1 idea B). It is read once at the top of a run and then
 * costs header height for the rest of it, so the summary line carries what a
 * returning user actually needs and the body stays one click away.
 *
 * Open state is local and unpersisted, unlike the activity log's: restoring it
 * would re-expand for a returning user the block the paragraph above collapses
 * for them.
 */
export function InitialPromptPanel({
  featureDescription,
  variant = 'card',
}: {
  featureDescription: string;
  variant?: 'card' | 'chip';
}) {
  const [open, setOpen] = useState(false);
  const chip = variant === 'chip';

  const disclosure = (
      <Disclosure
        title={chip ? 'Prompt' : 'Initial Prompt'}
        variant={variant}
        open={open}
        onOpenChange={setOpen}
        icon={<MessageSquareText className={`w-4 h-4 ${TONE_TEXT.violet}`} />}
        meta={
          open ? undefined : (
            <span
              data-testid="initial-prompt-summary"
              className={`text-xs font-mono text-slate-500 truncate ${chip ? 'max-w-[32ch]' : 'max-w-[48ch]'}`}
              title={featureDescription || undefined}
            >
              {promptSummary(featureDescription)}
            </span>
          )
        }
        bodyClassName="max-h-48 overflow-y-auto"
      >
        {/* The measure caps the *line*, not the card. It sat on the panel until
            now, so the title bar, the chevron and the summary were capped with
            the prose — which left a band of chrome ending two thirds of the way
            across a wide window with nothing beside it. `PROSE_CH` rather than a
            second spelling of 96: it is exported for this and both call sites
            used to re-state the number instead. */}
        <div
          style={{ maxWidth: `${PROSE_CH}ch` }}
          className="p-4 text-sm text-slate-300 font-mono whitespace-pre-wrap leading-relaxed"
        >
          {featureDescription
            ? featureDescription
            : <span className="text-slate-500 italic">No initial prompt was recorded for this run.</span>}
        </div>
      </Disclosure>
  );

  if (chip) return disclosure;
  return <div className="px-6 py-4 bg-[var(--bg-app)] border-b border-white/5">{disclosure}</div>;
}
