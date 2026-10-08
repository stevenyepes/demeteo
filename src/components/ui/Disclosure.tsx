import React, { useId } from 'react';
import { ChevronRight } from 'lucide-react';

export interface DisclosureProps {
  title: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  icon?: React.ReactNode;
  /** Right-hand side of the trigger row — a run id, a sync affordance, a
   *  status chip. Rendered *beside* the trigger button, never inside it, so a
   *  control placed here is reachable by keyboard instead of being swallowed by
   *  the button it would be nested in. */
  meta?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  /** The body brings its own padding: the panels adopting this disagree about
   *  it (a prompt reads at card padding, a log at list padding) and a Tailwind
   *  utility appended here cannot reliably override one baked into the
   *  primitive. */
  bodyClassName?: string;
  /** `card` is a full-width bar of its own. `chip` is a pill that sits in a
   *  row of siblings inside a `flex-wrap` parent: the wrapper is
   *  `display: contents`, so the pill and its body are *children of that row*,
   *  and the body's `order-last basis-full` drops it below every pill instead
   *  of splitting the row at the pill that opened it. That is the reason the
   *  variant lives here rather than in a caller — the body still unmounts when
   *  closed, which is the whole point of this primitive. */
  variant?: 'card' | 'chip';
}

/**
 * Collapsible section whose body **unmounts** when closed (UI_REDESIGN_PLAN
 * §5.1). That is the whole reason this exists rather than a `hidden` class:
 * the panels it wraps are expensive and *active* — `ActivityPanel` tails the
 * runner over the tunnel, the initial prompt renders a long markdown block —
 * and CSS hiding leaves both running behind a collapsed summary line.
 *
 * Open state is the caller's, and so is whether it survives a restart.
 *
 * Height is not animated. The content is of unknown size, so an animated
 * `max-height` would force layout every frame over a list that may hold
 * hundreds of rows; the body uses the app's one-shot `.animate-fade-in`, which
 * the `prefers-reduced-motion` block in `src/App.css` already collapses to
 * instant.
 */
export function Disclosure({
  title,
  open,
  onOpenChange,
  icon,
  meta,
  children,
  className = '',
  bodyClassName = '',
  variant = 'card',
}: DisclosureProps): React.ReactElement {
  const baseId = useId();
  const triggerId = `${baseId}-trigger`;
  const bodyId = `${baseId}-body`;

  if (variant === 'chip') {
    return (
      <div data-testid="disclosure" data-open={open ? 'true' : 'false'} className={`contents ${className}`}>
        <div
          className={`flex h-8 max-w-full min-w-0 items-center gap-2 rounded-full border pr-3 text-xs transition-colors ${
            open ? 'border-violet-500/40 bg-violet-500/10' : 'border-white/10 bg-white/[0.03] hover:bg-white/[0.06]'
          }`}
        >
          <button
            type="button"
            id={triggerId}
            data-testid="disclosure-trigger"
            aria-expanded={open}
            aria-controls={open ? bodyId : undefined}
            onClick={() => onOpenChange(!open)}
            className="flex h-full shrink-0 items-center gap-2 rounded-full pl-3 text-left"
          >
            {icon && <span className="shrink-0 flex items-center">{icon}</span>}
            <span className={`font-medium ${open ? 'text-violet-200' : 'text-slate-200'}`}>{title}</span>
          </button>
          {meta && <div className="flex min-w-0 items-center gap-2 overflow-hidden">{meta}</div>}
        </div>
        {open && (
          <div
            id={bodyId}
            data-testid="disclosure-body"
            role="region"
            aria-labelledby={triggerId}
            className={`glass-panel animate-fade-in order-last basis-full overflow-hidden ${bodyClassName}`}
          >
            {children}
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      data-testid="disclosure"
      data-open={open ? 'true' : 'false'}
      className={`glass-panel overflow-hidden ${className}`}
    >
      <div className="flex items-center gap-2">
        <button
          type="button"
          id={triggerId}
          data-testid="disclosure-trigger"
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          onClick={() => onOpenChange(!open)}
          className="flex-1 min-w-0 px-4 py-3 flex items-center gap-2 text-left hover:bg-white/[0.02] transition-colors"
        >
          {icon && <span className="shrink-0 flex items-center">{icon}</span>}
          <span className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider truncate">
            {title}
          </span>
          <ChevronRight
            aria-hidden="true"
            className={`w-4 h-4 shrink-0 text-slate-500 transition-transform ${open ? 'rotate-90' : ''}`}
          />
        </button>
        {meta && <div className="shrink-0 pr-4 flex items-center gap-2 min-w-0">{meta}</div>}
      </div>
      {open && (
        <div
          id={bodyId}
          data-testid="disclosure-body"
          role="region"
          aria-labelledby={triggerId}
          className={`animate-fade-in border-t border-white/5 ${bodyClassName}`}
        >
          {children}
        </div>
      )}
    </div>
  );
}
