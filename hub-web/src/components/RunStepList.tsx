import { Chip } from '../../../src/components/ui/Chip';
import { runStatusMeta } from '../../../src/lib/runStatus';

export type ListedStep = { stepId: string; title: string | null; status: string };

interface RunStepListProps {
  label: string;
  steps: ListedStep[];
  onSelect: (stepId: string) => void;
}

/** The id leads each button's name, since a title is optional and two steps may share one. */
export function RunStepList({ label, steps, onSelect }: RunStepListProps) {
  return (
    <ul aria-label={label} className="flex flex-col gap-1">
      {steps.map(({ stepId, title, status }) => (
        <li key={stepId}>
          <button
            type="button"
            onClick={() => onSelect(stepId)}
            className="flex w-full flex-wrap items-center gap-x-3 gap-y-1 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-white/5"
          >
            <span className="font-mono text-sm text-slate-200 break-all">{stepId}</span>
            {title !== null && <span className="text-sm text-slate-400 break-words">{title}</span>}
            <span className="ml-auto">
              <Chip status={status}>{runStatusMeta(status).label}</Chip>
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}
