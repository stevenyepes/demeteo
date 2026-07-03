import { Workflow } from 'lucide-react';
import type { WorkflowSummary } from '../../types';

export interface CreateZeroWorkflowStepProps {
  workflows: ReadonlyArray<WorkflowSummary>;
  workflowId: string;
  onWorkflowChange: (id: string) => void;
}

/**
 * Step 8 — pick a workflow. Mirrors the picker that lives inside
 * `StartFeatureModal` but rendered as a full wizard step so the
 * choice gets its own focused surface.
 */
export function CreateZeroWorkflowStep(props: CreateZeroWorkflowStepProps) {
  const { workflows, workflowId, onWorkflowChange } = props;
  if (workflows.length === 0) {
    return (
      <p className="text-xs text-slate-400 font-mono">
        No workflows available. Add one in the Workflows view.
      </p>
    );
  }
  return (
    <div className="space-y-2">
      {workflows.map((w) => (
        <button
          key={w.id}
          type="button"
          onClick={() => onWorkflowChange(w.id)}
          className={`w-full text-left p-3 rounded-lg border transition-all ${
            workflowId === w.id
              ? 'bg-violet-500/10 border-violet-500/50 text-violet-100'
              : 'bg-black/30 border-white/10 text-slate-300 hover:border-white/20'
          }`}
        >
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2 min-w-0">
              <Workflow className="w-4 h-4 text-violet-400 shrink-0" />
              <span className="font-medium truncate">{w.name}</span>
            </div>
            <span className="text-[10px] font-mono text-slate-500 shrink-0">v{w.version}</span>
          </div>
          <p className="mt-1 text-[11px] text-slate-400 line-clamp-2">{w.description}</p>
        </button>
      ))}
    </div>
  );
}
