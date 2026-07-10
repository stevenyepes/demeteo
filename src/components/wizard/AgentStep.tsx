import { Cpu } from 'lucide-react';
import type { CreateProjectStepPayload } from '../../types';

export interface AgentStepProps {
  agentKinds: ReadonlyArray<string>;
  value: string;
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'agent' }>) => void;
}

/**
 * Step 5 — Agent. Pick a coding agent harness. The available kinds are
 * supplied by the caller from the backend `list_agents` catalog (the single
 * source of truth); any kind outside the registered set is rejected by the
 * Rust command. Emits the matching `{ step: 'agent', kind }` payload upward.
 */
export function AgentStep({ agentKinds, value, onSubmit }: AgentStepProps) {
  return (
    <div className="space-y-4" data-testid="wizard-step-agent">
      <label
        htmlFor="wizard-agent-select"
        className="text-[11px] font-mono text-slate-400 uppercase tracking-widest flex items-center gap-1.5"
      >
        <Cpu className="w-3 h-3" /> Which coding agent should run the feature?
      </label>
      <select
        id="wizard-agent-select"
        value={value}
        onChange={(e) => onSubmit({ step: 'agent', kind: e.target.value })}
        data-testid="wizard-agent-select-input"
        className="w-full bg-[#08090c] border border-white/10 rounded-lg py-3 px-3 text-sm text-white capitalize focus:outline-none focus:border-violet-500/50"
      >
        <option value="">Pick a harness…</option>
        {agentKinds.map((k) => (
          <option key={k} value={k}>
            {k.replace(/-/g, ' ')}
          </option>
        ))}
      </select>
      <p className="text-[10px] text-slate-500 font-mono">
        Selection persists as the project's default agent.
      </p>
    </div>
  );
}