/**
 * The card React Flow draws for one workflow node. Dark neon glassmorphism to
 * match the rest of the app; the kind icon + type chip make a graph scannable
 * without opening panels (the anti-"identical boxes" rule, PRD §6.3). Read-only
 * in P2.1 — the live status overlay (pulse/duration/failure tint) and the
 * drill-down panel land in P2.2/P2.3.
 */
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { nodeTypeMeta } from '../types';
import { TONE_TEXT, TONE_CHIP } from '../../../lib/runStatus';
import type { WorkflowFlowNode } from '../flowGraph';

const HANDLE_CLASS = '!h-2 !w-2 !border-slate-600 !bg-slate-800';

export function WorkflowNode({ data, selected }: NodeProps<WorkflowFlowNode>) {
  const meta = nodeTypeMeta(data.nodeType);
  const Icon = meta.icon;

  return (
    <div
      className={[
        'group flex min-w-[200px] max-w-[280px] items-center gap-3 rounded-xl border px-3.5 py-2.5',
        'bg-slate-900/70 backdrop-blur-sm transition-shadow',
        selected
          ? 'border-cyan-400/70 shadow-[0_0_0_1px_rgba(34,211,238,0.4),0_0_18px_rgba(34,211,238,0.25)]'
          : 'border-slate-700/60 shadow-lg shadow-black/30 hover:border-slate-600',
      ].join(' ')}
    >
      <Handle type="target" position={Position.Top} className={HANDLE_CLASS} />

      <div
        className={[
          'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border',
          TONE_CHIP[meta.tone],
        ].join(' ')}
      >
        <Icon className={`h-4 w-4 ${TONE_TEXT[meta.tone]}`} aria-hidden />
      </div>

      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium text-slate-100" title={data.title}>
          {data.title}
        </div>
        <div className={`text-[11px] font-medium uppercase tracking-wide ${TONE_TEXT[meta.tone]}`}>
          {meta.label}
        </div>
      </div>

      <Handle type="source" position={Position.Bottom} className={HANDLE_CLASS} />
    </div>
  );
}
