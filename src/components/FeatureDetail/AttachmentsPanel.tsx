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
    <div className="px-6 py-4 bg-[#08090c] border-b border-white/5">
      <div className="max-w-[96ch] flex flex-col gap-2">
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
