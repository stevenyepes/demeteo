import { ArrowRight, Check, ChevronLeft, Cpu, Rocket } from 'lucide-react';
import { Button } from './Button';

export interface CreateZeroStepFooterProps {
  step: 'name' | 'provider' | 'machine' | 'agent' | 'strategy' | 'description' | 'workflow';
  canProceed: boolean;
  reason: string;
  onBack: () => void;
  onNext: () => void;
}

/**
 * Footer row — Back / Next. Hidden on the bootstrap and launching
 * screens because those auto-progress. The primary CTA changes its
 * label to mirror the action; its colour stays the accent throughout.
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
  const statusText =
    reason || (
      isApprove ? 'Approve to continue' :
      isLast ? 'Launch the feature' :
      'Ready'
    );
  return (
    <div className="flex items-center justify-between pt-2 border-t border-white/5">
      <Button variant="ghost" onClick={onBack} disabled={step === 'name'}>
        <ChevronLeft className="w-4 h-4" /> Back
      </Button>
      <div className="flex items-center gap-3">
        <span className="text-[10px] text-slate-500 font-mono">{statusText}</span>
        <Button variant="primary" onClick={onNext} disabled={!canProceed}>
          {ctaLabel}
        </Button>
      </div>
    </div>
  );
}
