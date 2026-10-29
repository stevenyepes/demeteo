import { Check, ChevronRight, X } from 'lucide-react';
import { Fragment } from 'react';

import type { WorkflowDefinitionV2 } from '../../../src/components/canvas/types';
import { runStatusMeta, TONE_CHIP, TONE_TEXT } from '../../../src/lib/runStatus';
import { pipelineColumns } from '../lib/runView';
import type { StatusMap } from '../lib/useRunDetail';

interface RunPipelineStripProps {
  workflow: Pick<WorkflowDefinitionV2, 'nodes' | 'edges'>;
  statusByNode: StatusMap;
}

/**
 * The run graph flattened to one line, for a preview too narrow for the
 * canvas. A step the instance has not reported stays neutral: the mirror
 * says nothing about it, which is not the same as `pending`.
 */
export function RunPipelineStrip({ workflow, statusByNode }: RunPipelineStripProps) {
  const titles = new Map(workflow.nodes.map((node) => [node.id, node.title]));
  const columns = pipelineColumns(workflow);
  return (
    <ol aria-label="Pipeline" className="flex flex-wrap items-center gap-1.5 text-xs">
      {columns.map((ids, index) => (
        <Fragment key={ids.join(' ')}>
          {index > 0 && <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-slate-600" />}
          <li className="flex flex-col gap-1.5">
            {ids.map((id) => (
              <StripNode key={id} title={titles.get(id) ?? id} status={statusByNode[id]?.status ?? null} />
            ))}
          </li>
        </Fragment>
      ))}
    </ol>
  );
}

function StripNode({ title, status }: { title: string; status: string | null }) {
  if (status === null) {
    return (
      <span className="inline-flex h-7 items-center rounded-md border border-hairline px-2.5 text-slate-400">
        {title}
      </span>
    );
  }
  const meta = runStatusMeta(status);
  const settledWell = meta.tone === 'emerald';
  return (
    <span
      title={meta.label}
      className={`inline-flex h-7 items-center gap-1.5 rounded-md border px-2.5 ${
        settledWell ? 'border-hairline text-slate-200' : TONE_CHIP[meta.tone]
      }`}
    >
      {settledWell ? (
        <Check aria-hidden="true" className={`size-3.5 ${TONE_TEXT.emerald}`} />
      ) : meta.tone === 'ruby' ? (
        <X aria-hidden="true" className="size-3.5" />
      ) : (
        <span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
      )}
      {title}
      <span className="sr-only">, {meta.label}</span>
    </span>
  );
}
