import { FileWarning } from 'lucide-react';

import { TONE_TEXT } from '../../../lib/runStatus';
import type { ConflictFile } from '../../../types';
import { EmptyHint } from '../../canvas/nodePanel/EmptyHint';

/**
 * The unmerged paths, as git named them.
 *
 * An empty list is rendered as its own sentence rather than hidden: the
 * porcelain read that fills it answers empty on any transport error
 * (`crate::domain::sync_failure`), so "no files" here means "we could not read
 * them", never "the conflict is small".
 */
export function ConflictFileList({
  files,
  onOpenPath,
}: {
  files: ConflictFile[];
  onOpenPath: (filePath: string) => void;
}) {
  if (files.length === 0) {
    return <EmptyHint>git named no paths. That is a read that failed, not a conflict with no files.</EmptyHint>;
  }

  return (
    <ul data-testid="conflict-files" className="space-y-1">
      {files.map((file) => (
        <li key={file.path}>
          <button
            type="button"
            onClick={() => onOpenPath(file.path)}
            title={`Open ${file.path}`}
            className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition hover:bg-white/5"
          >
            <FileWarning className={`h-3.5 w-3.5 shrink-0 ${TONE_TEXT.ruby}`} />
            <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-slate-200">
              {file.path}
            </span>
            <span className="shrink-0 text-[10px] uppercase tracking-wider text-slate-500">
              {file.kind}
            </span>
          </button>
        </li>
      ))}
    </ul>
  );
}

export default ConflictFileList;
