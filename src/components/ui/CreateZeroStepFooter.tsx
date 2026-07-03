import { ArrowRight, Check, ChevronLeft, Cpu, Rocket } from 'lucide-react';

export interface CreateZeroStepFooterProps {
  step: 'name' | 'provider' | 'machine' | 'agent' | 'strategy' | 'description' | 'workflow';
  canProceed: boolean;
  reason: string;
  onBack: () => void;
  onNext: () => void;
}

/**
 * Footer row — Back / Next. Hidden on the bootstrap and launching
 * screens because those auto-progress. The primary CTA changes
 * label + colour to mirror the action: violet/cyan for navigation,
 * emerald for "Launch feature".
 */
export function CreateZeroStepFooter(props: CreateZeroStepFooterProps) {
  const { step, canProceed, reason, onBack, onNext } = props;
  const isLast = step === 'workflow';
  const isApprove = step === 'strategy';
  const isBootstrap = step === 'agent';
  const ctaLabel =
    isBootstrap ? (<><Cpu className="w-3.5 h-3.5" /> Create &amp; bootstrap</>) :
    isApprove ? (<><Check className="w-3.5 h-3.5" /> Approve &amp; continue</>) :
    isLast ? (<><Rocket className="w-3.5 h-3.5" /> Launch feature</>) :
    (<>Next <ArrowRight className="w-3.5 h-3.5" /></>);
  const ctaClass = isLast
    ? 'bg-emerald-600 hover:bg-emerald-500 text-white shadow-[0_0_15px_rgba(16,185,129,0.3)]'
    : 'bg-cyan-600 hover:bg-cyan-500 text-white shadow-[0_0_15px_rgba(6,182,212,0.3)]';
  const statusText =
    reason || (
      isApprove ? 'Approve to continue' :
      isLast ? 'Launch the feature' :
      'Ready'
    );
  return (
    <div className="flex items-center justify-between pt-2 border-t border-white/5">
      <button
        type="button"
        onClick={onBack}
        disabled={step === 'name'}
        className="px-4 py-2 text-xs font-medium text-slate-400 hover:text-white transition-colors disabled:opacity-30 disabled:cursor-not-allowed flex items-center gap-1.5"
      >
        <ChevronLeft className="w-4 h-4" /> Back
      </button>
      <div className="flex items-center gap-3">
        <span className="text-[10px] text-slate-500 font-mono">{statusText}</span>
        <button
          type="button"
          onClick={onNext}
          disabled={!canProceed}
          className={`px-5 py-2 text-xs font-bold rounded-lg transition-all flex items-center gap-1.5 ${
            canProceed ? ctaClass : 'bg-white/5 text-slate-600 cursor-not-allowed'
          }`}
        >
          {ctaLabel}
        </button>
      </div>
    </div>
  );
}
