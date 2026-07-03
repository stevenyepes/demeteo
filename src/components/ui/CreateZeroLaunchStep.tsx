import { AlertTriangle, Rocket, RotateCw } from 'lucide-react';

export interface CreateZeroLaunchStepProps {
  launching: boolean;
  errorMessage: string | null;
  onRetry: () => void;
}

/**
 * Step 9 — launching indicator. Spinner while the start_feature call
 * is in flight; surfaces any rejection inline with a retry CTA.
 */
export function CreateZeroLaunchStep(props: CreateZeroLaunchStepProps) {
  const { launching, errorMessage, onRetry } = props;
  if (launching) {
    return (
      <div className="py-8 flex flex-col items-center text-center gap-3">
        <Rocket className="w-10 h-10 text-violet-400 animate-pulse-glow" />
        <p className="text-sm text-slate-200 font-mono">Launching feature…</p>
        <p className="text-[11px] text-slate-500 font-mono">This usually takes a few seconds.</p>
      </div>
    );
  }
  if (errorMessage) {
    return (
      <div className="py-8 flex flex-col items-center text-center gap-3">
        <AlertTriangle className="w-10 h-10 text-ruby-400" />
        <p className="text-sm text-ruby-200 font-mono">Launch failed: {errorMessage}</p>
        <button
          type="button"
          onClick={onRetry}
          className="px-4 py-2 text-xs font-medium bg-violet-600 hover:bg-violet-500 text-white rounded-md transition-all flex items-center gap-1.5"
        >
          <RotateCw className="w-3.5 h-3.5" /> Retry launch
        </button>
      </div>
    );
  }
  return (
    <p className="py-8 text-center text-sm text-emerald-300 font-mono">Launched.</p>
  );
}
