import { ActivityIndicator, AgentBadge, MachineDot } from 'demeteo';

const Legend = ({ label, children }: { label: string; children: React.ReactNode }) => (
  <div className="flex items-center gap-3">
    <span className="w-6 flex justify-center">{children}</span>
    <span className="text-xs font-mono text-slate-400">{label}</span>
  </div>
);

/** The three states, in the order of visual weight the component intends:
 *  a decision out-shouts a wait, which out-shouts work in progress. */
export const States = () => (
  <div className="flex flex-col gap-3">
    <Legend label="working — violet spinner">
      <ActivityIndicator activity="working" />
    </Legend>
    <Legend label="awaiting_input — steady amber">
      <ActivityIndicator activity="awaiting_input" />
    </Legend>
    <Legend label="awaiting_approval — pulsing ruby">
      <ActivityIndicator activity="awaiting_approval" />
    </Legend>
    <Legend label="null — no mark at all">
      <ActivityIndicator activity={null} />
    </Legend>
  </div>
);

/** Where it actually lives: beside the agent badge in a session row. */
export const InASessionRow = () => (
  <div className="flex flex-col gap-1 w-full max-w-sm">
    {[
      { name: 'demeteo · master', agent: 'claude-code', activity: 'working', machine: 'local' },
      { name: 'demeteo · feat/ssh-pool', agent: 'opencode', activity: 'awaiting_approval', machine: 'build-01' },
      { name: 'demeteo · docs', agent: 'hermes', activity: 'awaiting_input', machine: 'build-01' },
      { name: 'scratch shell', agent: null, activity: null, machine: 'local' },
    ].map((s) => (
      <div
        key={s.name}
        className="flex items-center gap-2 px-3 py-2 rounded-lg bg-white/[0.02] border border-white/5"
      >
        <MachineDot machineId={s.machine} machineLabel={s.machine} pulse={s.activity === 'working'} />
        <span className="text-xs text-slate-300 flex-1 truncate">{s.name}</span>
        <AgentBadge agentKind={s.agent} compact />
        <ActivityIndicator activity={s.activity as never} />
      </div>
    ))}
  </div>
);
