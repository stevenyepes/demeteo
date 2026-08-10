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
}: DisclosureProps): React.ReactElement {
  const baseId = useId();
  const triggerId = `${baseId}-trigger`;
  const bodyId = `${baseId}-body`;

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
