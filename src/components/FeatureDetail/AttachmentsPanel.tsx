import { AttachmentChip } from '../AttachmentChip';
import type { AttachedFile } from '../../lib/attachments';

/** Read-only chips with hover metadata + click-to-view (sub-3). */
export function AttachmentsPanel({
  attachments,
  onView,
}: {
  attachments: AttachedFile[];
  onView: (attachmentId: string) => void;
}) {
  if (attachments.length === 0) return null;
  return (
    <div className="px-6 py-4 bg-[var(--bg-app)] border-b border-white/5">
      {/* No reading measure here: these are chips, and a cap that stops them
          wrapping at the width prose wants leaves rows breaking early against a
          band of empty space. `PROSE_CH` belongs to the prompt's body. */}
      <div className="flex flex-col gap-2">
        <div className="text-xs text-violet-400 font-bold uppercase tracking-widest flex items-center gap-2">
          Attachments
          <span className="text-[10px] text-slate-500 font-mono normal-case tracking-tight">
            {attachments.length} file{attachments.length === 1 ? '' : 's'}
          </span>
        </div>
        <div className="flex flex-wrap gap-2">
          {attachments.map((a) => (
            <AttachmentChip
              key={a.id}
              attachment={a}
              onClick={(id) => onView(id)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
