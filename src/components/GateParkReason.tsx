import React from 'react';
import { FileText, ShieldAlert } from 'lucide-react';
import { Button } from './ui/Button';

interface GateParkReasonProps {
  reason: string;
  /** The implementation report to offer beside the reason, when a step before
   *  the gate produced one. */
  report: { path: string; stepId: string } | null;
  onOpenReport: (path: string, stepId: string) => void;
}

/**
 * Why a step stopped to ask. The step's `error_message` is the only copy of
 * the question, so without this the modal renders a generic "review the
 * artifact" over a park that may have produced nothing to review, and the
 * person is asked to decide something nobody told them.
 *
 * A validate park turns on evidence, and the evidence is the implementation
 * report — which the picker below lists among a dozen rows. The button puts it
 * one click from the question that asks about it.
 */
export const GateParkReason: React.FC<GateParkReasonProps> = ({ reason, report, onOpenReport }) => (
  <div
    data-testid="gate-park-reason"
    className="p-4 rounded-lg bg-amber-500/[0.04] border border-amber-500/20 text-sm text-slate-300 leading-relaxed space-y-3"
  >
    <div className="flex items-center justify-between gap-3">
      <div className="text-amber-300 font-semibold flex items-center gap-1.5 text-xs">
        <ShieldAlert className="w-3.5 h-3.5" /> Why the run stopped here
      </div>
      {report && (
        <Button
          size="sm"
          data-testid="gate-open-implementation-report"
          onClick={() => onOpenReport(report.path, report.stepId)}
          title="Open the implementation report: what each ticket's agent claimed, and what Demeteo observed it run"
        >
          <FileText className="w-3.5 h-3.5" /> Implementation report
        </Button>
      )}
    </div>
    <p className="whitespace-pre-wrap">{reason}</p>
  </div>
);
