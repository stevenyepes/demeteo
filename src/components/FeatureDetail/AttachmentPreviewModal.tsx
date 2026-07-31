import { Paperclip, X } from 'lucide-react';
import { Modal } from '../ui/Modal';
import type { AttachedFile } from '../../lib/attachments';

/**
 * For image/* attachments the bytes are fetched via the `attachment_read`
 * IPC and rendered inline as a `data:<mime>;base64,…` URL. Non-image mimes
 * (pdf / txt / md / json) skip the IPC and show a generic glass metadata
 * panel instead — no inline renderer for those kinds in v1.
 */
export function AttachmentPreviewModal({
  attachments,
  viewingAttachmentId,
  previewUrl,
  onClose,
}: {
  attachments: AttachedFile[];
  viewingAttachmentId: string | null;
  previewUrl: string | null;
  onClose: () => void;
}) {
  if (!viewingAttachmentId) return null;
  const attachment = attachments.find((a) => a.id === viewingAttachmentId);
  if (!attachment) return null;
  const isImage = attachment.mime.startsWith('image/');
  return (
    <Modal
      onClose={onClose}
      backdropClassName="bg-black/70"
      className="bg-[#0d0f14] border border-white/10 rounded-2xl p-0 max-w-3xl w-full mx-4 shadow-[0_0_40px_rgba(0,0,0,0.5)] overflow-hidden"
    >
      <div className="px-5 py-3 border-b border-white/5 flex items-center justify-between">
        <div className="flex items-center gap-3 min-w-0">
          <span className="font-mono text-xs text-cyan-300 truncate" title={attachment.source_filename}>
            {attachment.source_filename}
          </span>
          <span className="text-[10px] font-mono uppercase tracking-wider px-1.5 py-0.5 rounded-md border border-violet-500/30 bg-violet-500/10 text-violet-300">
            {attachment.mime}
          </span>
          <span className="text-[10px] font-mono text-slate-500">
            {(attachment.size / 1024).toFixed(1)} KB
          </span>
        </div>
        <button
          onClick={onClose}
          className="p-1.5 text-slate-400 hover:text-white transition"
          aria-label="Close"
        >
          <X className="w-4 h-4" />
        </button>
      </div>
      <div className="p-5 max-h-[70vh] overflow-auto bg-[#08090c]">
        {isImage && previewUrl ? (
          <img
            src={previewUrl}
            alt={attachment.source_filename}
            className="w-full h-auto rounded-lg border border-white/5"
          />
        ) : (
          <div
            data-testid="attachment-metadata-panel"
            className="rounded-xl border border-violet-500/10 bg-[rgba(18,22,30,0.75)] backdrop-blur-xl p-6 flex flex-col gap-4"
          >
            <div className="flex items-center gap-3 min-w-0">
              <Paperclip className="w-5 h-5 text-violet-300 shrink-0" />
              <span
                className="font-display text-sm font-bold text-white tracking-wide truncate"
                title={attachment.source_filename}
              >
                {attachment.source_filename}
              </span>
              <span className="text-[10px] font-mono uppercase tracking-wider px-1.5 py-0.5 rounded-md border border-violet-500/30 bg-violet-500/10 text-violet-300 shrink-0">
                {attachment.mime}
              </span>
              <span className="text-[10px] font-mono text-slate-500 shrink-0">
                {(attachment.size / 1024).toFixed(1)} KB
              </span>
            </div>
            <div className="text-[10px] font-mono text-slate-500 break-all">
              <span className="uppercase tracking-wider text-slate-600">sha256 </span>
              <span className="text-slate-400">{attachment.sha256}</span>
            </div>
            <div className="text-xs text-slate-400 italic border-t border-white/5 pt-3">
              No inline preview available for this file type.
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}
