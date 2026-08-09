import { AgentBadge } from 'demeteo';

/** Every agent kind the terminal panel knows about (lib/agents.ts). */
export const AllAgents = () => (
  <div className="flex flex-wrap items-center gap-2">
    <AgentBadge agentKind="claude-code" />
    <AgentBadge agentKind="opencode" />
    <AgentBadge agentKind="hermes" />
    <AgentBadge agentKind="codex" />
    <AgentBadge agentKind="pi" />
  </div>
);

/** Compact — icon only, for the narrow session list. Label moves to `title`. */
export const Compact = () => (
  <div className="flex items-center gap-2">
    <AgentBadge agentKind="claude-code" compact />
    <AgentBadge agentKind="opencode" compact />
    <AgentBadge agentKind="hermes" compact />
  </div>
);

/** An unknown kind is title-cased rather than dropped; null renders nothing. */
export const UnknownAndEmpty = () => (
  <div className="flex items-center gap-3">
    <AgentBadge agentKind="some-future-agent" />
    <span className="text-xs font-mono text-slate-500">null →</span>
    <AgentBadge agentKind={null} />
    <span className="text-xs font-mono text-slate-500">(nothing)</span>
  </div>
);

/** In place: the badge marks which terminals are running an agent. */
export const InATerminalList = () => (
  <div className="flex flex-col gap-1 w-full max-w-sm">
    {[
      { name: 'demeteo · master', agent: 'claude-code' },
      { name: 'demeteo · feat/windows', agent: 'codex' },
      { name: 'build-01 · shell', agent: null },
    ].map((t) => (
      <div key={t.name} className="flex items-center gap-2 px-3 py-2 rounded-lg bg-white/[0.02] border border-white/5">
        <span className="text-xs text-slate-300 flex-1 truncate">{t.name}</span>
        <AgentBadge agentKind={t.agent} />
      </div>
    ))}
  </div>
);
