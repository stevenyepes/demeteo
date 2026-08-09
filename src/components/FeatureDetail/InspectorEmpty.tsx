import { MousePointerClick, SearchX, Workflow } from 'lucide-react';

import { INSPECTOR_SURFACE } from '../ui/Inspector';
import { TONE_TEXT } from '../../lib/runStatus';
import type { InspectorEmptyReason } from '../../lib/inspectorTarget';

/**
 * What the inspector says when it has nothing to inspect.
 *
 * The pane never hides (UI_REDESIGN_PLAN §7), so this is on screen for a
 * measurable share of the time a run is open — and one generic "nothing
 * selected" for all three reasons is what makes a permanent empty pane read as
 * a broken one. Each reason gets its own sentence because each is a different
 * situation for the reader: an invitation, progress, and an explanation of a
 * link that outlived its run.
 */
const COPY: Record<InspectorEmptyReason, { icon: typeof Workflow; title: string; body: string }> = {
  'no-steps': {
    icon: Workflow,
    title: 'No steps yet',
    body: 'This run has not been decomposed into steps. They appear here as the workflow starts them.',
  },
  'no-selection': {
    icon: MousePointerClick,
    title: 'No step selected',
    body: 'Pick a step in the run to see its attempts, output and actions.',
  },
  'stale-selection': {
    icon: SearchX,
    title: 'That step is gone',
    body: 'The step this view was pointing at is no longer part of the run. Pick another one.',
  },
};

export function InspectorEmpty({
  reason,
  className = '',
}: {
  reason: InspectorEmptyReason;
  className?: string;
}) {
  const { icon: Icon, title, body } = COPY[reason];
  return (
    <div data-testid="inspector-empty" data-reason={reason} className={`${INSPECTOR_SURFACE} ${className}`}>
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-8 text-center">
        <Icon className={`h-6 w-6 ${TONE_TEXT.slate}`} />
        <h3 className="font-heading text-sm font-bold uppercase tracking-wider text-slate-300">
          {title}
        </h3>
        <p className="max-w-[36ch] text-xs leading-relaxed text-slate-500">{body}</p>
      </div>
    </div>
  );
}

export default InspectorEmpty;
