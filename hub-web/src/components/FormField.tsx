import type { ReactNode } from 'react';

export const LABEL_CLASS = 'text-xs font-medium text-slate-300';

/** A labelled control with its hint and error below it, both tied to it by `aria-describedby` through `describedBy`. */
export function FormField({
  id,
  label,
  hint,
  error,
  children,
}: {
  id: string;
  label: string;
  hint?: ReactNode;
  error?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={id} className={LABEL_CLASS}>
        {label}
      </label>
      {children}
      {hint && (
        <p id={`${id}-hint`} className="text-xs text-slate-400">
          {hint}
        </p>
      )}
      {error && (
        <p id={`${id}-error`} className="text-xs text-ruby-300">
          {error}
        </p>
      )}
    </div>
  );
}

/** The `aria-describedby` a control inside `FormField` takes. */
export function describedBy(id: string, hint: boolean, error: string | undefined): string | undefined {
  const ids = [hint ? `${id}-hint` : null, error ? `${id}-error` : null].filter((held) => held !== null);
  return ids.length === 0 ? undefined : ids.join(' ');
}
