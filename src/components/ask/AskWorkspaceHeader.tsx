import React from 'react';
import { Plus, Settings } from 'lucide-react';

import { formatCost, formatTokens } from '../../lib/utils';
import type { AskThread } from '../../types';
import { Chip } from '../ui/Chip';
import { Metric, MetricStrip } from '../ui/MetricStrip';
import { AskThreadSwitcher } from './AskThreadSwitcher';

interface AskWorkspaceHeaderProps {
  thread: AskThread;
  projectId: string;
  onSelectThread: (threadId: string) => void;
  onNewThread: () => void;
  onOpenSettings: () => void;
}

export function AskWorkspaceHeader({
  thread,
  projectId,
  onSelectThread,
  onNewThread,
  onOpenSettings,
}: AskWorkspaceHeaderProps): React.ReactElement {
  return (
    <header className="flex shrink-0 items-center justify-between gap-6 border-b border-white/5 bg-[#0d0f14]/60 px-6 py-3.5">
      <div className="flex min-w-0 flex-col gap-1.5">
        <p className="m-0 font-mono text-[11px] text-slate-500">Ask</p>
        <div className="flex min-w-0 items-center gap-3">
          <h1 className="m-0 truncate font-heading text-xl font-bold tracking-tight text-white">
            {thread.title}
          </h1>
          <Chip size="sm" tone="cyan">
            {thread.agent_kind}
          </Chip>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-4">
        <MetricStrip variant="inset">
          <Metric label="Turns" value={String(thread.turn_count)} />
          <Metric label="Spend" value={formatCost(thread.cost_usd)} tone="emerald" />
          <Metric label="Tokens" value={formatTokens(thread.tokens)} tone="cyan" />
        </MetricStrip>

        <AskThreadSwitcher
          projectId={projectId}
          activeThreadId={thread.id}
          onSelect={onSelectThread}
        />

        <button
          type="button"
          data-testid="ask-open-settings"
          aria-label="Thread settings"
          onClick={onOpenSettings}
          className="btn-secondary inline-flex items-center gap-2"
        >
          <Settings className="h-3.5 w-3.5" aria-hidden="true" />
        </button>

        <button
          type="button"
          data-testid="ask-new-thread"
          onClick={onNewThread}
          className="btn-primary inline-flex items-center gap-2"
        >
          <Plus className="h-3.5 w-3.5" aria-hidden="true" />
          New thread
        </button>
      </div>
    </header>
  );
}

export default AskWorkspaceHeader;
