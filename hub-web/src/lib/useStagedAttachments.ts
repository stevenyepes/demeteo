import { useRef, useState } from 'react';

import { type StagedAttachment, uploadAttachment } from './attachments';

export interface StagedAttachments {
  files: File[];
  /** Takes the picker's whole next list; a file no longer in it loses its staged id. */
  setFiles: (files: File[]) => void;
  /**
   * Uploads, one at a time and in file order, every file not yet staged, and
   * resolves to all the ids in file order. It rejects on the first failed
   * upload, so a dispatch never names part of the list; the files staged
   * before it keep their ids, and the next call does not upload them again.
   */
  stage: (instanceId: string) => Promise<string[]>;
  /** For `attachment_unavailable`: an id the Hub has spent or expired would be refused again. */
  unstage: () => void;
}

export function useStagedAttachments(): StagedAttachments {
  const [files, setHeld] = useState<File[]>([]);
  const staged = useRef(new Map<File, StagedAttachment>());

  const setFiles = (next: File[]) => {
    for (const file of staged.current.keys()) {
      if (!next.includes(file)) staged.current.delete(file);
    }
    setHeld(next);
  };

  const stage = async (instanceId: string) => {
    const ids: string[] = [];
    for (const file of files) {
      let held = staged.current.get(file);
      if (held === undefined) {
        held = await uploadAttachment(instanceId, file);
        staged.current.set(file, held);
      }
      ids.push(held.attachmentId);
    }
    return ids;
  };

  return { files, setFiles, stage, unstage: () => staged.current.clear() };
}
