import { Cpu, Loader2 } from 'lucide-react';
import { HarnessModelPicker, type ModelOption } from './HarnessModelPicker';

export interface CreateZeroAgentStepProps {
  agentKinds: ReadonlyArray<string>;
  models: ReadonlyArray<ModelOption>;
  modelsLoading: boolean;
  agentKind: string;
  model: string;
  onAgentKindChange: (kind: string) => void;
  onModelChange: (model: string) => void;
  onClear: () => void;
}

/**
 * Step 4 — pick a coding agent and (optionally) a model. Wraps the
 * shared {@link HarnessModelPicker} with the wizard-specific hint
 * that the probe is machine-scoped.
 */
export function CreateZeroAgentStep(props: CreateZeroAgentStepProps) {
  const { agentKinds, models, modelsLoading, agentKind, model,
    onAgentKindChange, onModelChange, onClear } = props;
  return (
    <div className="space-y-4">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block">
        Which coding agent should run the feature?
      </label>
      <HarnessModelPicker
        agentKinds={agentKinds as string[]}
        models={models as ModelOption[]}
        modelsLoading={modelsLoading}
        agentKind={agentKind}
        model={model}
        onAgentKindChange={onAgentKindChange}
        onModelChange={onModelChange}
        onClear={onClear}
        agentPlaceholder="Pick a harness"
        modelPlaceholder="Agent default model"
      />
      {modelsLoading && (
        <p className="text-[11px] text-cyan-300 font-mono flex items-center gap-1.5">
          <Loader2 className="w-3 h-3 animate-spin" />
          Probing {agentKind} on the selected machine…
        </p>
      )}
      <p className="text-[10px] text-slate-500 font-mono">
        The probe is machine-scoped — model availability can differ between local and remote machines.
      </p>
      <p className="text-[10px] text-slate-600 font-mono flex items-center gap-1.5">
        <Cpu className="w-3 h-3" /> Selection persists as the project's default agent/model.
      </p>
    </div>
  );
}
