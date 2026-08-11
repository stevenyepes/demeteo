import React from 'react';
import { AlertCircle, Loader2 } from 'lucide-react';

import { runStatusMeta, TONE_TEXT } from '../../../lib/runStatus';
import type { StepAttempt } from '../../../types';
import { EmptyHint } from './EmptyHint';
import { classLabel, formatCost, formatMs } from './format';

/** The per-attempt history from `step_attempts` (class · cost · duration ·
 *  applied rule), so a retry loop is legible instead of collapsed onto the one
 *  row the timeline overwrites. */
export function AttemptTable({
  hasExecution,
  attempts,
  loading,
  error,
}: {
  hasExecution: boolean;
  attempts: StepAttempt[];
  loading: boolean;
  error: string | null;
}) {
  return (
    <div>
      <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
        Attempt history
      </div>
      {!hasExecution ? (
        <EmptyHint>This node hasn&apos;t started yet.</EmptyHint>
      ) : loading && attempts.length === 0 ? (
        <div className="flex items-center gap-2 py-6 text-xs text-slate-500">
          <Loader2 className="h-4 w-4 animate-spin text-violet-400" /> Loading attempts…
        </div>
      ) : error ? (
        <div className="flex items-start gap-2 rounded-lg border border-rose-500/20 bg-rose-950/20 p-3 text-xs text-rose-300">
          <AlertCircle className="mt-px h-4 w-4 shrink-0 text-rose-400" />
          <span>{error}</span>
        </div>
      ) : attempts.length === 0 ? (
        <EmptyHint>No attempt rows recorded.</EmptyHint>
      ) : (
        <div className="overflow-hidden rounded-xl border border-white/5">
          <table className="w-full border-collapse text-left text-xs">
            <thead className="bg-white/[0.02] text-[10px] uppercase tracking-wider text-slate-500">
              <tr>
                <Th>#</Th>
                <Th>Status</Th>
                <Th>Class</Th>
                <Th>Cost</Th>
                <Th>Duration</Th>
                <Th>Rule</Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/[0.03]">
              {attempts.map((a) => {
                const aMeta = runStatusMeta(a.status);
                return (
                  <tr key={a.attempt_no} className="text-slate-300">
                    <Td className="font-mono text-slate-400">{a.attempt_no}</Td>
                    <Td>
                      <span className={`font-semibold ${TONE_TEXT[aMeta.tone]}`}>{aMeta.label}</span>
                    </Td>
                    <Td className="text-slate-400">
                      {a.error_class ? classLabel(a.error_class) : '—'}
                    </Td>
                    <Td className="font-mono">{formatCost(a.cost_usd)}</Td>
                    <Td className="font-mono">{formatMs(a.wall_clock_ms)}</Td>
                    <Td className="font-mono text-[10px] text-slate-400">{a.applied_rule ?? '—'}</Td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function Th({ children }: { children: React.ReactNode }) {
  return <th className="px-3 py-2 font-semibold">{children}</th>;
}

function Td({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return <td className={`px-3 py-2 ${className}`}>{children}</td>;
}

export default AttemptTable;
