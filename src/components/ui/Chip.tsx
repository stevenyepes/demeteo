/**
 * Chip — the one small pill (UI redesign plan §5.1). Status pills, workflow
 * pills and transport pills were four separate spellings that had drifted in
 * padding, radius, font size and casing; the whole point of this primitive is
 * that a call site can no longer choose any of those, nor a colour: tone comes
 * from `lib/runStatus.ts` (audit F27), so a new surface cannot re-open the
 * status-colour drift by hand-writing `bg-emerald-500/10 text-…`.
 *
 * Casing and typography are deliberately not props. A chip is uppercase mono
 * everywhere or the pills stop reading as one family, which is the drift this
 * component exists to end.
 */

import type { ReactNode } from 'react';

import { runStatusMeta, TONE_CHIP, type RunStatusTone } from '../../lib/runStatus';

export type ChipSize = 'sm' | 'md';

interface ChipBaseProps {
  /** Leading glyph, sized by the caller (`<Cpu className="w-3 h-3" />`). */
  icon?: ReactNode;
  /** Defaults to the resolved status's liveness; `pulse` forces it on. */
  dot?: boolean;
  /**
   * Defaults to `runStatusMeta().active`. Only ever animates the dot: a
   * pulsing box that holds text re-rasterizes the text every frame, which
   * is the incident recorded above `@keyframes pulse-glow` in `src/App.css`.
   */
  pulse?: boolean;
  size?: ChipSize;
  title?: string;
  /**
   * CSS length that caps the pill and ellipsizes the label — for
   * caller-supplied names (a workflow) that have no length bound. Inline
   * rather than a class prop so the escape hatch can only ever set a width,
   * never smuggle in a colour.
   */
  maxWidth?: string;
  className?: string;
}

interface StatusChipProps extends ChipBaseProps {
  /** Resolved through `runStatusMeta` for tone, liveness and default label. */
  status: string;
  tone?: RunStatusTone;
  children?: ReactNode;
}

/** `status` stays optional-and-possibly-`undefined` rather than `never`: the
 *  call sites this replaces (`ProjectHome`, `RemoteRunInbox`) hold a
 *  `string | undefined` status, and a `status?: undefined` arm rejects that
 *  without narrowing first — a type that forces a cast at every migration is
 *  the type being wrong, not the caller. An explicit `tone` and `children`
 *  are what this arm actually requires. */
interface ToneChipProps extends ChipBaseProps {
  status?: string | undefined;
  tone: RunStatusTone;
  children: ReactNode;
}

export type ChipProps = StatusChipProps | ToneChipProps;

const SIZE_CLASS: Record<ChipSize, string> = {
  sm: 'gap-1 px-2 py-0.5 text-[10px]',
  md: 'gap-1.5 px-2.5 py-0.5 text-xs',
};

export function Chip(props: ChipProps): React.ReactElement {
  const { icon, dot, pulse, size = 'md', title, maxWidth, className = '', children } = props;

  const meta = props.status === undefined ? undefined : runStatusMeta(props.status);
  const tone = props.tone ?? meta?.tone ?? 'slate';
  const pulsing = pulse ?? meta?.active ?? false;
  const showDot = dot ?? pulsing;

  return (
    <span
      data-testid="chip"
      data-tone={tone}
      title={title}
      style={maxWidth ? { maxWidth } : undefined}
      className={`inline-flex items-center shrink-0 rounded border font-mono uppercase tracking-wide ${
        SIZE_CLASS[size]
      } ${TONE_CHIP[tone]} ${maxWidth ? 'overflow-hidden' : ''} ${className}`}
    >
      {showDot && (
        <span
          aria-hidden="true"
          data-testid="chip-dot"
          className={`w-1.5 h-1.5 rounded-full shrink-0 bg-current ${
            pulsing ? 'animate-pulse motion-reduce:animate-none' : ''
          }`}
        />
      )}
      {icon && (
        <span aria-hidden="true" className="shrink-0 inline-flex items-center">
          {icon}
        </span>
      )}
      <span data-testid="chip-label" className={maxWidth ? 'truncate' : ''}>
        {children ?? meta?.label}
      </span>
    </span>
  );
}
