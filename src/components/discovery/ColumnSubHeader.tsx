import React from 'react';

interface ColumnSubHeaderProps {
  /** Omitted when the left slot carries a control instead of a name. */
  title?: string;
  /** Chips or a segmented control, right-aligned. */
  children?: React.ReactNode;
  /** The inspector's bar rides over its own scroller. */
  sticky?: boolean;
  left?: React.ReactNode;
  className?: string;
}

/**
 * The 38 px bar every column of the Discovery workspace wears
 * (`DISCOVERY_UI_SPEC.md` §6.2, which asks for it once rather than three
 * times).
 */
export function ColumnSubHeader({
  title,
  children,
  sticky = false,
  left,
  className = '',
}: ColumnSubHeaderProps): React.ReactElement {
  return (
    <div
      data-testid="column-sub-header"
      className={`flex h-[38px] shrink-0 items-center justify-between gap-3 border-b border-white/5 bg-[#12161e]/60 px-4 ${
        sticky ? 'sticky top-0 z-[2]' : ''
      } ${className}`}
    >
      <div className="flex min-w-0 items-center gap-2">
        {title && (
          <span className="truncate font-heading text-xs font-medium text-slate-400">{title}</span>
        )}
        {left}
      </div>
      {children && <div className="flex shrink-0 items-center gap-1.5">{children}</div>}
    </div>
  );
}

export default ColumnSubHeader;
