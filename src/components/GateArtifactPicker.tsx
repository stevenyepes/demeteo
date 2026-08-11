import { ArtifactRow } from './ArtifactRow';
import { Chip } from './ui/Chip';
import { listStepArtifacts } from '../lib/stepArtifacts';
import type { StepExecution } from '../types';

interface GateArtifactPickerProps {
  steps: StepExecution[];
  gateStepIndex: number;
  selectedArtifactPath: string | null;
  onSelectArtifact: (path: string, stepTitle: string) => void;
}

/**
 * Every step strictly before the gate with something listable, one
 * row-group each. A step whose `listStepArtifacts` comes back empty (e.g. a
 * `command`/baseline step) is omitted entirely rather than rendered as an
 * empty group — see spec §7.3.
 *
 * Selection is fully controlled by the caller (`GateView`), matching the
 * `useArtifactSelection` pattern already used elsewhere in the app: this
 * component owns no local selection state.
 */
export function GateArtifactPicker({
  steps,
  gateStepIndex,
  selectedArtifactPath,
  onSelectArtifact,
}: GateArtifactPickerProps) {
  const reviewable = steps
    .filter((step) => step.step_index < gateStepIndex)
    .map((step) => ({ step, listed: listStepArtifacts(step).listed }))
    .filter(({ listed }) => listed.length > 0)
    .sort((a, b) => a.step.step_index - b.step.step_index);

  if (reviewable.length === 0) {
    return (
      <div className="px-1 font-mono text-[10px] text-slate-600">
        No reviewable artifacts from earlier steps.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {reviewable.map(({ step, listed }) => (
        <div key={step.id} className="space-y-2">
          <div className="flex items-center gap-2">
            <span className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
              {step.step_id}
            </span>
            <Chip status={step.status} size="sm" />
          </div>
          <div className="space-y-2">
            {listed.map((path) => (
              <ArtifactRow
                key={path}
                path={path}
                selected={selectedArtifactPath === path}
                onSelect={() => onSelectArtifact(path, step.step_id)}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

export default GateArtifactPicker;
