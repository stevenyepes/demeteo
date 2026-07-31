import { useEffect, useState } from 'react';
import { listAttachments, readAttachment, type AttachedFile } from '../../lib/attachments';
import { bytesToDataUrl } from './bytesToDataUrl';

/**
 * Per-feature attachments (sub-3 brief). Fetched once per feature id via
 * `feature_list_attachments`, which the orchestrator already wires in
 * `src-tauri/src/lib.rs` — this only consumes the result. Rendered as
 * read-only chips below the Initial Prompt panel; click opens a Modal
 * preview, hover surfaces a soft tooltip with mime + size + sha256.
 *
 * Click-to-view fires the `attachment_read` IPC for image/* attachments so
 * the preview Modal can render an out-of-session file (one that arrived
 * through Tauri drag-and-drop with no browser `File` handle). Non-image mimes
 * (pdf / txt / md / json) skip the round-trip entirely and render a metadata
 * panel instead.
 */
export function useAttachmentPreview(featureId: string) {
  const [attachments, setAttachments] = useState<AttachedFile[]>([]);
  const [viewingAttachmentId, setViewingAttachmentId] = useState<string | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);

  useEffect(() => {
    if (!viewingAttachmentId) {
      setPreviewUrl(null);
      return;
    }
    const attachment = attachments.find((a) => a.id === viewingAttachmentId);
    if (!attachment) {
      setPreviewUrl(null);
      return;
    }
    if (!attachment.mime.startsWith('image/')) {
      setPreviewUrl(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const { mime, bytes } = await readAttachment(featureId, attachment.id);
        if (!cancelled) setPreviewUrl(bytesToDataUrl(mime, bytes));
      } catch {
        if (!cancelled) setPreviewUrl(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [viewingAttachmentId, attachments, featureId]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const list = await listAttachments(featureId);
        if (!cancelled) setAttachments(list);
      } catch (err) {
        // Soft failure — the section will just render empty. Errors
        // here are non-actionable for the user (no Rust panics, only
        // IPC validation issues).
        if (!cancelled) setAttachments([]);
        console.warn('listAttachments failed:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [featureId]);

  const closePreview = () => {
    setViewingAttachmentId(null);
    setPreviewUrl(null);
  };

  return { attachments, viewingAttachmentId, setViewingAttachmentId, previewUrl, closePreview };
}
