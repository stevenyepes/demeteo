import { useState } from 'react';
import { MessageSquareText } from 'lucide-react';

import { Disclosure } from '../ui/Disclosure';
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
 * Open state is deliberately local and unpersisted: Phase 6 owns session
 * persistence for every Disclosure at once, and a bespoke store here is work
 * it would delete.
 */
export function InitialPromptPanel({ featureDescription }: { featureDescription: string }) {
  const [open, setOpen] = useState(false);

  return (
    <div className="px-6 py-4 bg-[var(--bg-app)] border-b border-white/5">
      <Disclosure
        title="Initial Prompt"
        open={open}
        onOpenChange={setOpen}
        icon={<MessageSquareText className={`w-4 h-4 ${TONE_TEXT.violet}`} />}
        meta={
          open ? undefined : (
            <span
              data-testid="initial-prompt-summary"
              className="text-xs font-mono text-slate-500 truncate max-w-[48ch]"
              title={featureDescription || undefined}
            >
              {promptSummary(featureDescription)}
            </span>
          )
        }
        className="max-w-[96ch]"
        bodyClassName="max-h-48 overflow-y-auto"
      >
        <div className="p-4 text-sm text-slate-300 font-mono whitespace-pre-wrap leading-relaxed">
          {featureDescription
            ? featureDescription
            : <span className="text-slate-500 italic">No initial prompt was recorded for this run.</span>}
        </div>
      </Disclosure>
    </div>
  );
}
