import type { ReactNode } from 'react';

interface PageHeaderProps {
  title: string;
  /** Above the title: where the page sits — "Personal fleet", or a link back to its list. */
  eyebrow?: ReactNode;
  /** Below the title, one mono line of counts or identifiers. */
  summary?: ReactNode;
  /** The page's own actions, right-aligned and wrapping under the title on a narrow window. */
  actions?: ReactNode;
}

/** Every Hub view opens with this, so the one `h1` on a page is always here. */
export function PageHeader({ title, eyebrow, summary, actions }: PageHeaderProps) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-4">
      <div className="flex min-w-0 flex-col gap-2">
        {eyebrow !== undefined && <div className="font-mono text-xs text-slate-400 break-all">{eyebrow}</div>}
        <h1 className="font-heading text-3xl font-semibold text-white">{title}</h1>
        {summary !== undefined && <div className="font-mono text-sm text-slate-400 break-words">{summary}</div>}
      </div>
      {actions !== undefined && <div className="flex flex-wrap gap-3">{actions}</div>}
    </header>
  );
}
