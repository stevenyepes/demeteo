import { type ChangeEvent, type ClipboardEvent, type DragEvent, useId, useState } from 'react';

import { MAX_ATTACHMENTS_PER_DISPATCH } from '../lib/attachments';

interface AttachmentPickerProps {
  files: File[];
  onChange: (files: File[]) => void;
  /** Set while the form uploads or dispatches, when the list must hold still. */
  disabled?: boolean;
}

const chipKeys = new WeakMap<File, number>();
let nextChipKey = 0;

function chipKey(file: File): number {
  let key = chipKeys.get(file);
  if (key === undefined) {
    key = nextChipKey++;
    chipKeys.set(file, key);
  }
  return key;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/**
 * Not `src/components/AttachmentDropzone.tsx`: it takes drops through
 * `@tauri-apps/api/webview`, which the browser has no counterpart for. This one
 * only collects `File`s; the form uploads them before it dispatches.
 *
 * The count cap is checked here because the Hub's would only refuse the
 * eleventh upload after ten had been staged. The byte limit is not: it is Hub
 * config the browser cannot read, so an oversized file is left to the upload's
 * `413 attachment_too_large`. Nor does the copy promise a file's name reaches
 * the prompt — the upload is raw bytes, and the Hub keeps no name.
 */
export function AttachmentPicker({ files, onChange, disabled = false }: AttachmentPickerProps) {
  const inputId = useId();
  const noteId = useId();
  const [refused, setRefused] = useState<string | null>(null);

  const add = (incoming: File[]) => {
    if (disabled || incoming.length === 0) return;
    const room = Math.max(0, MAX_ATTACHMENTS_PER_DISPATCH - files.length);
    const accepted = incoming.slice(0, room);
    const turnedAway = incoming.slice(room);
    setRefused(
      turnedAway.length === 0
        ? null
        : `A dispatch carries at most ${MAX_ATTACHMENTS_PER_DISPATCH} files, so these were not added: ${turnedAway
            .map((file) => file.name)
            .join(', ')}.`,
    );
    if (accepted.length > 0) onChange([...files, ...accepted]);
  };

  const remove = (file: File) => {
    setRefused(null);
    onChange(files.filter((held) => held !== file));
  };

  const onPick = (event: ChangeEvent<HTMLInputElement>) => {
    add(Array.from(event.target.files ?? []));
    event.target.value = '';
  };

  const onDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    add(Array.from(event.dataTransfer.files));
  };

  const onPaste = (event: ClipboardEvent<HTMLDivElement>) => {
    const pasted = Array.from(event.clipboardData.files);
    if (pasted.length === 0) return;
    event.preventDefault();
    add(pasted);
  };

  return (
    <div
      role="group"
      aria-label="Attachments"
      aria-describedby={noteId}
      tabIndex={disabled ? -1 : 0}
      onDragOver={(event) => event.preventDefault()}
      onDrop={onDrop}
      onPaste={onPaste}
      className="nested-card flex flex-col gap-3 p-4 focus-visible:outline-2 focus-visible:outline-cyan-400"
    >
      <div className="flex flex-wrap items-center gap-3">
        <label
          htmlFor={inputId}
          className={`btn-secondary text-xs focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-cyan-400 ${disabled ? 'cursor-not-allowed opacity-40' : 'cursor-pointer'}`}
        >
          Add files
          <input id={inputId} type="file" multiple disabled={disabled} onChange={onPick} className="sr-only" />
        </label>
        <span className="text-xs text-slate-400">
          or drop or paste them here · {files.length} of {MAX_ATTACHMENTS_PER_DISPATCH}
        </span>
      </div>
      {files.length > 0 && (
        <ul className="flex flex-wrap gap-2">
          {files.map((file) => (
            <li
              key={chipKey(file)}
              className="inline-flex items-center gap-2 rounded-lg border border-white/10 bg-black/40 py-1 pr-1 pl-2 text-xs"
            >
              <span className="font-mono text-slate-200 break-all">{file.name}</span>
              <span className="font-mono text-slate-500">{formatSize(file.size)}</span>
              <button
                type="button"
                onClick={() => remove(file)}
                disabled={disabled}
                aria-label={`Remove ${file.name}`}
                className="rounded px-1.5 text-slate-400 transition-colors hover:text-ruby-300 focus-visible:outline-2 focus-visible:outline-cyan-400 disabled:cursor-not-allowed disabled:opacity-40"
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}
      {refused && (
        <p role="alert" className="text-xs text-ruby-300">
          {refused}
        </p>
      )}
      <p id={noteId} className="text-xs text-slate-500">
        Files are uploaded encrypted, fetched once by the instance, then deleted. The Hub keeps no file name, and
        refuses a file over its size limit when it uploads.
      </p>
    </div>
  );
}
