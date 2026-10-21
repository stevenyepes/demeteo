import React from 'react';
import { Sparkles } from 'lucide-react';

import { agentLabel } from '../../lib/agents';

export interface AgentBadgeProps {
  /** Agent kind (`"claude-code"`, `"opencode"`, …). Renders nothing when
   *  null/undefined — a plain shell carries no badge. */
  agentKind?: string | null;
  /** Compact variant: icon only, label available via `title`. Used in the
   *  narrow session list where horizontal space is tight. */
  compact?: boolean;
  className?: string;
}

/**
 * A small neutral pill marking a terminal that is running a coding agent
 * (spec: "if it is running a coding agent, show a label"). It names *what*
 * runs, which is not a state, so it takes no hue: the machine dot beside it
 * already spends one on *where*.
 */
function AgentBadgeImpl({
  agentKind,
  compact = false,
  className = '',
}: AgentBadgeProps): React.ReactElement | null {
  const label = agentLabel(agentKind);
  if (!label) return null;

  return (
    <span
      data-testid="agent-badge"
      data-agent-kind={agentKind ?? ''}
      title={`Running ${label}`}
      className={[
        'inline-flex items-center gap-1 rounded-full border border-white/10',
        'bg-white/5 text-slate-300 font-mono shrink-0',
        compact ? 'px-1 py-0.5' : 'px-1.5 py-0.5 text-[9px]',
        className,
      ].join(' ')}
    >
      <Sparkles className="w-2.5 h-2.5 shrink-0" aria-hidden="true" />
      {!compact && <span className="uppercase tracking-wide">{label}</span>}
    </span>
  );
}

export const AgentBadge = React.memo(AgentBadgeImpl);
