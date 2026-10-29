import type React from 'react';

/**
 * The app's one button (AGENTS.md §4, DESIGN.md drift 2.3).
 *
 * Fifty primaries had been spelled at their call sites, each choosing its own
 * violet, glow and padding, so the primary action read differently on every
 * screen. The variant names a role, never a hue: the fill of `primary` is the
 * accent, and there is deliberately no emerald or cyan variant, because a
 * status hue on a control says the action *is* that state. Approving a gate is
 * the primary action of the gate, so it is `primary` like any other.
 *
 * `className` is for layout (`w-full`, `ml-auto`, `shrink-0`). A colour or
 * padding passed there fights the variant's class at equal specificity and
 * wins or loses by stylesheet order, so add a variant instead.
 *
 * Hover is spelled `not-disabled:hover:` so a disabled button does not light
 * up under the pointer. Not `enabled:`, which never matches the `<a>` that
 * `buttonClasses` styles. Focus is the global `:focus-visible` ring in `App.css`.
 */

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'danger-subtle';

/** `md` is the dialog and toolbar default; `lg` a status screen's one action. */
export type ButtonSize = 'sm' | 'md' | 'lg';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  ref?: React.Ref<HTMLButtonElement>;
}

const BASE =
  'inline-flex items-center justify-center gap-1.5 rounded-md border font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-40';

const VARIANT: Record<ButtonVariant, string> = {
  primary: 'border-transparent bg-accent text-white not-disabled:hover:bg-accent-hover',
  secondary:
    'border-white/10 bg-white/5 text-slate-200 not-disabled:hover:bg-white/10 not-disabled:hover:text-white',
  ghost: 'border-transparent text-slate-400 not-disabled:hover:bg-white/5 not-disabled:hover:text-white',
  // Hover steps darker, not lighter: white on ruby-500 is under 4.5:1.
  danger: 'border-transparent bg-ruby-600 text-white not-disabled:hover:bg-ruby-700',
  'danger-subtle':
    'border-ruby-500/30 text-ruby-400 not-disabled:hover:bg-ruby-500/10 not-disabled:hover:text-ruby-300',
};

const SIZE: Record<ButtonSize, string> = {
  sm: 'px-2.5 py-1 text-xs',
  md: 'px-4 py-2 text-xs',
  lg: 'px-5 py-2.5 text-sm',
};

/** The classes alone, for the rare control that must not be a `<button>` (a
 *  file-input `<label>`, an `<a>`). */
export function buttonClasses(variant: ButtonVariant = 'secondary', size: ButtonSize = 'md'): string {
  return `${BASE} ${VARIANT[variant]} ${SIZE[size]}`;
}

export function Button({
  variant = 'secondary',
  size = 'md',
  type = 'button',
  className = '',
  ref,
  ...rest
}: ButtonProps): React.ReactElement {
  return (
    <button
      ref={ref}
      type={type}
      className={`${buttonClasses(variant, size)} ${className}`}
      {...rest}
    />
  );
}

export default Button;
