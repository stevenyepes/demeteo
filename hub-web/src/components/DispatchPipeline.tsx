import { useMemo, useState } from 'react';

import { WorkflowCanvas } from '../../../src/components/canvas/WorkflowCanvas';
import { definitionOf, type StepOverrideDraft, toStepOverrides } from '../lib/dispatchForm';
import type { WorkflowChoice } from '../lib/fleet';
import { StepOverridePanel } from './StepOverridePanel';

interface DispatchPipelineProps {
  workflow: WorkflowChoice;
  overrides: Record<string, StepOverrideDraft>;
  onOverridesChange: (next: Record<string, StepOverrideDraft>) => void;
  /** The form's default agent, which a step's own agent inherits. */
  defaultAgentKind: string;
}

/**
 * The definition is memoised on the workflow alone and overridden steps are
 * marked through `highlightedNodeIds`: a new `definition` identity makes the
 * canvas drop its layout and run elk again, which on every keystroke in the
 * panel would re-lay the graph under the user's cursor.
 */
export function DispatchPipeline({ workflow, overrides, onOverridesChange, defaultAgentKind }: DispatchPipelineProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const definition = useMemo(() => definitionOf(workflow), [workflow]);
  const overridden = useMemo(() => new Set(toStepOverrides(overrides).map((entry) => entry.step_id)), [overrides]);

  if (definition === null) {
    return (
      <p className="nested-card px-4 py-3 text-sm text-slate-400">
        Per-step overrides are unavailable for this workflow: the instance reported no graph for it, so every step runs
        on the defaults above.
      </p>
    );
  }

  const selected = workflow.graph?.nodes.find((node) => node.id === selectedId) ?? null;

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_20rem]">
      <div className="nested-card h-80 overflow-hidden">
        <div className="panel-field h-full w-full">
          <WorkflowCanvas
            definition={definition}
            mode="run"
            onNodeActivate={setSelectedId}
            selectedNodeId={selectedId}
            highlightedNodeIds={overridden}
          />
        </div>
      </div>
      {selected === null ? (
        <p className="text-xs text-slate-400">Select a step on the graph to pin its agent, model or effort.</p>
      ) : (
        <section aria-label="Step override">
          <StepOverridePanel
            node={selected}
            override={overrides[selected.id] ?? {}}
            defaultAgentKind={defaultAgentKind}
            onChange={(next) => onOverridesChange({ ...overrides, [selected.id]: next })}
            onReset={() => {
              const { [selected.id]: _reset, ...rest } = overrides;
              onOverridesChange(rest);
            }}
          />
        </section>
      )}
    </div>
  );
}
