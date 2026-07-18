import React from 'react';

export interface ScrollAreaProps extends React.HTMLAttributes<HTMLDivElement> {
  children?: React.ReactNode;
}

/** Styled vertical scroll container for the rail's project/workspace list and
 *  the Terminals session list (TERMINALS_VIEW_SPEC §5, §9).
 *
 *  `overflow-y-auto` scrolls internally; `overscroll-contain` stops scroll
 *  chaining from leaking into an underlying xterm surface; `min-h-0` lets the
 *  area shrink inside a flex column instead of pushing siblings off-screen.
 *
 *  `scrollbar-width: thin` (inline) plus the stable `demeteo-scrollarea`
 *  className give future global CSS a hook for WebKit `::-webkit-scrollbar`
 *  styling — no global stylesheet is added here. */
export const ScrollArea = React.forwardRef<HTMLDivElement, ScrollAreaProps>(
  ({ children, className = '', style, ...rest }, ref) => (
    <div
      ref={ref}
      data-testid="scroll-area"
      className={`demeteo-scrollarea overflow-y-auto overscroll-contain min-h-0 ${className}`}
      style={{ scrollbarWidth: 'thin', ...style }}
      {...rest}
    >
      {children}
    </div>
  ),
);

ScrollArea.displayName = 'ScrollArea';
