import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { UploadCloud, FileWarning, FilePlus2 } from "lucide-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  addAttachment,
  computeLocalSha256,
  type AttachedFile,
  type AttachmentInput,
} from "../lib/attachments";
import { AttachmentChip } from "./AttachmentChip";

/**
 * A staged attachment kept in local memory until the parent commits it
 * to a feature via `feature_add_attachment`. The Rust handler requires
 * a `feature_id` that only exists once a feature has been launched,
 * so pre-launch composers stage files here and resolve them after
 * the launch call returns.
 */
export interface LaunchStageEntry {
  /** Lowercase hex SHA-256 of the picked bytes. */
  sha256: string;
  /** Sanitized filename for the chip label. */
  name: string;
  /** Original filename, kept verbatim for the user. */
  source_filename: string;
  /** IANA mime — picked from the File object or inferred from the path. */
  mime: string;
  /** Byte length, used to render the chip's size label. */
  size: number;
  /** Local data URL — produced from the browser File via FileReader. */
  previewUrl: string | null;
  /** Browser `File` handle (only when picked via `<input type="file">`). */
  file: File | null;
  /** Absolute disk path. Used by the Rust command when committing. */
  sourcePath: string | null;
}

interface AttachmentDropzoneProps {
  /** `launch` keeps entries local until {@link onCommitLaunch} runs;
   *  `direct` calls `addAttachment` immediately per pick. */
  mode: "launch" | "direct";
  /** Required for `direct` mode (the feature the attachment is added to). */
  featureId?: string;
  /** Visible label, e.g. "Attachments" / "Add files". */
  label?: string;
  /** Compact variant for collapsed chip rows (no border, no padding). */
  compact?: boolean;
  /** Limit visible size of the chip row. */
  maxChips?: number;

  // -- direct-mode IPC mirror (used by GateView / FeatureDetail add flows) --
  /** Called when a new attachment is created server-side in `direct` mode. */
  onAdded?: (a: AttachedFile) => void;
  /** Called on the optimistic local removal of a `direct`-mode attachment. */
  onRemoved?: (id: string) => void;

  // -- launch-mode staging (used by StartFeatureModal / ProjectHome) --
  /** Currently staged entries. Parent owns the source of truth. */
  stageEntries?: LaunchStageEntry[];
  /** Replace the entire stage list (delete / reorder / external flows). */
  onChangeStage?: (next: LaunchStageEntry[]) => void;

  // -- soft errors that surface inline (not via a toast) --
  onError?: (message: string) => void;
}

/**
 * Glass-surface panel with drag-and-drop + click-to-pick behavior.
 *
 * In `direct` mode the dropzone calls the `feature_add_attachment`
 * Tauri command as soon as a file is dropped or picked. In `launch`
 * mode the dropzone is a local-file staging area; the parent must
 * call `feature_add_attachment` for each staged entry once the
 * launched feature id is known.
 *
 * Drag-and-drop is delivered through Tauri's
 * `getCurrentWebview().onDragDropEvent` API (`@tauri-apps/api` v2) —
 * the OS path comes back as a string, the bytes are read by the Rust
 * command. Click-to-pick falls back to a hidden `<input type="file">`
 * which DOES yield a browser `File` (used to render preview
 * thumbnails via FileReader).
 *
 * Clipboard paste is intentionally NOT supported in v1 (see spec §0
 * decision #5).
 */
export const AttachmentDropzone: React.FC<AttachmentDropzoneProps> = ({
  mode,
  featureId,
  label,
  compact,
  maxChips,
  onAdded,
  onRemoved,
  stageEntries,
  onChangeStage,
  onError,
}) => {
  const [isHovered, setIsHovered] = useState(false);
  const [directAttachments, setDirectAttachments] = useState<AttachedFile[]>([]);
  const [isPicking, setIsPicking] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // -- drag-and-drop wiring (Tauri v2 webview API) ------------------------
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    (async () => {
      try {
        const webview = getCurrentWebview();
        unlisten = await webview.onDragDropEvent(async (event) => {
          if (cancelled) return;
          const payload = event.payload;
          if (payload.type === "over") {
            setIsHovered(true);
            return;
          }
          if (payload.type === "leave") {
            setIsHovered(false);
            return;
          }
          // payload.type === "drop"
          setIsHovered(false);
          const paths = payload.paths ?? [];
          if (paths.length === 0) return;
          await ingestPaths(paths);
        });
      } catch (err) {
        // Non-Tauri environment (storybook / unit test mount) — silently
        // disable drag-drop; the click-to-pick path still works.
        console.warn("AttachmentDropzone: drag-drop unavailable", err);
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, featureId, stageEntries]);

  // -- click-to-pick via <input type="file" /> ----------------------------
  const openPicker = useCallback(() => {
    if (isPicking) return;
    setIsPicking(true);
    inputRef.current?.click();
  }, [isPicking]);

  const onPickerChange = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      setIsPicking(false);
      const files = e.target.files;
      if (!files || files.length === 0) return;
      await ingestFiles(Array.from(files));
      // Reset so the same file can be re-picked after a remove.
      e.target.value = "";
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mode, featureId, stageEntries],
  );

  // -- shared ingest ------------------------------------------------------
  const ingestFiles = useCallback(
    async (files: File[]) => {
      for (const file of files) {
        try {
          if (mode === "direct") {
            await ingestOneDirect({ kind: "file", file });
          } else {
            await ingestOneLaunch({ kind: "file", file });
          }
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          onError?.(message);
        }
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mode, featureId, stageEntries],
  );

  const ingestPaths = useCallback(
    async (paths: string[]) => {
      for (const sourcePath of paths) {
        const lower = sourcePath.toLowerCase();
        const mime = guessMime(lower);
        const sourceFilename = sourcePath.split(/[\\/]/).pop() ?? sourcePath;
        try {
          if (mode === "direct") {
            await ingestOneDirect({ kind: "path", sourcePath, sourceFilename, mime });
          } else {
            await ingestOneLaunch({ kind: "path", sourcePath, sourceFilename, mime });
          }
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          onError?.(message);
        }
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mode, featureId, stageEntries],
  );

  const ingestOneDirect = useCallback(
    async (input: AttachmentInput) => {
      if (!featureId) {
        throw new Error("AttachmentDropzone: featureId is required for direct mode");
      }
      const created = await addAttachment(featureId, input);
      setDirectAttachments((prev) => [
        ...prev.filter((p) => p.id !== created.id),
        created,
      ]);
      onAdded?.(created);
    },
    [featureId, onAdded],
  );

  const ingestOneLaunch = useCallback(
    async (input: AttachmentInput) => {
      if (!onChangeStage) {
        throw new Error(
          "AttachmentDropzone: onChangeStage is required for launch mode",
        );
      }
      const sourceFilename =
        input.kind === "file"
          ? input.file.name
          : (input.sourceFilename ?? input.sourcePath.split(/[\\/]/).pop() ?? "attachment");
      const mime =
        input.kind === "file"
          ? (input.file.type || guessMime(sourceFilename.toLowerCase()))
          : (input.mime ?? guessMime(sourceFilename.toLowerCase()));
      const size = input.kind === "file" ? input.file.size : 0;
      const sourcePath =
        input.kind === "path"
          ? input.sourcePath
          : (input.file as File & { path?: string }).path ?? null;
      const file = input.kind === "file" ? input.file : null;

      // Local sha256 for dedup + the future manifest key parity.
      const sha256 = file ? await computeLocalSha256(file) : "staged-" + randomId();

      const previewUrl =
        file && mime.startsWith("image/")
          ? await readDataUrl(file)
          : null;

      const entry: LaunchStageEntry = {
        sha256,
        name: sourceFilename,
        source_filename: sourceFilename,
        mime,
        size,
        previewUrl,
        file,
        sourcePath,
      };

      onChangeStage([...(stageEntries ?? []).filter((e) => e.sha256 !== sha256), entry]);
    },
    [onChangeStage, stageEntries],
  );

  // -- render the chip list ----------------------------------------------
  const visibleEntries: { key: string; entry: LaunchStageEntry; remove: () => void }[] = useMemo(() => {
    if (mode === "launch" && stageEntries) {
      return stageEntries.map((entry) => ({
        key: entry.sha256,
        entry,
        remove: () =>
          onChangeStage?.((stageEntries ?? []).filter((e) => e.sha256 !== entry.sha256)),
      }));
    }
    if (mode === "direct") {
      return directAttachments.map((a) => ({
        key: a.id,
        entry: {
          sha256: a.sha256,
          name: a.name,
          source_filename: a.source_filename,
          mime: a.mime,
          size: a.size,
          previewUrl: null,
          file: null,
          sourcePath: null,
        },
        remove: async () => {
          if (!featureId) return;
          try {
            const { removeAttachment } = await import("../lib/attachments");
            await removeAttachment(featureId, a.id);
            setDirectAttachments((prev) => prev.filter((p) => p.id !== a.id));
            onRemoved?.(a.id);
          } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            onError?.(message);
          }
        },
      }));
    }
    return [];
  }, [mode, stageEntries, directAttachments, featureId, onChangeStage, onRemoved, onError]);

  const visibleLimited =
    typeof maxChips === "number" ? visibleEntries.slice(0, maxChips) : visibleEntries;
  const hiddenCount = visibleEntries.length - visibleLimited.length;

  // -- compact render is just the chips (no panel) -----------------------
  if (compact) {
    return (
      <div className="flex flex-wrap items-center gap-1.5">
        {visibleLimited.length === 0 ? (
          <span className="text-[11px] font-mono text-slate-500 italic">No attachments</span>
        ) : (
          visibleLimited.map(({ key, entry, remove }) => (
            <AttachmentChip
              key={key}
              attachment={{
                id: key,
                name: entry.name,
                mime: entry.mime,
                sha256: entry.sha256,
                size: entry.size,
                source_filename: entry.source_filename,
              }}
              previewUrl={entry.previewUrl}
              compact
              onRemove={remove}
            />
          ))
        )}
        {hiddenCount > 0 && (
          <span className="text-[10px] font-mono text-slate-500">+{hiddenCount} more</span>
        )}
      </div>
    );
  }

  return (
    <div
      className={[
        "rounded-xl border transition-all",
        isHovered
          ? "border-cyan-400/60 bg-[rgba(18,22,30,0.85)]"
          : "border-white/10 bg-[rgba(18,22,30,0.75)]",
        "backdrop-blur-md p-3",
      ].join(" ")}
      onDragOver={(e) => {
        // Required for the drop event to fire in HTML5 fallback paths.
        e.preventDefault();
      }}
      onDragEnter={() => setIsHovered(true)}
      onDragLeave={(e) => {
        if (e.currentTarget === e.target) setIsHovered(false);
      }}
    >
      <input
        ref={inputRef}
        type="file"
        multiple
        className="hidden"
        onChange={onPickerChange}
        accept={ACCEPTED_TYPES}
        aria-hidden
      />
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={openPicker}
          className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg border border-violet-500/30 bg-violet-500/10 hover:bg-violet-500/20 text-violet-200 text-xs font-medium transition-colors"
        >
          <UploadCloud className="w-3.5 h-3.5" />
          {label ?? "Add files"}
        </button>
        <div className="flex-1 min-w-0 flex items-center gap-2 text-[11px] font-mono text-slate-400">
          <FilePlus2 className="w-3.5 h-3.5 text-slate-500 shrink-0" />
          <span className="truncate">
            or drop here · png / jpg / webp / gif / pdf / txt · max 100 MB each · 10 per feature
          </span>
        </div>
      </div>

      {visibleLimited.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-2">
          {visibleLimited.map(({ key, entry, remove }) => (
            <AttachmentChip
              key={key}
              attachment={{
                id: key,
                name: entry.name,
                mime: entry.mime,
                sha256: entry.sha256,
                size: entry.size,
                source_filename: entry.source_filename,
              }}
              previewUrl={entry.previewUrl}
              onRemove={remove}
            />
          ))}
          {hiddenCount > 0 && (
            <span className="text-[10px] font-mono text-slate-500 self-center">
              +{hiddenCount} more
            </span>
          )}
        </div>
      )}

      {visibleLimited.length === 0 && (
        <div className="mt-3 flex items-center gap-2 text-[11px] text-slate-500 font-mono">
          <FileWarning className="w-3.5 h-3.5 text-slate-600" />
          <span>No attachments yet. They will be referenced via [attachment -- &lt;name&gt;].</span>
        </div>
      )}
    </div>
  );
};

const ACCEPTED_TYPES = ".png,.jpg,.jpeg,.gif,.webp,.pdf,.txt,.md,.json";

function guessMime(lower: string): string {
  if (lower.endsWith(".png")) return "image/png";
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
  if (lower.endsWith(".gif")) return "image/gif";
  if (lower.endsWith(".webp")) return "image/webp";
  if (lower.endsWith(".pdf")) return "application/pdf";
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) return "text/markdown";
  if (lower.endsWith(".txt")) return "text/plain";
  if (lower.endsWith(".json")) return "application/json";
  return "application/octet-stream";
}

function readDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("FileReader failed"));
    reader.onload = () => {
      const out = reader.result;
      if (typeof out === "string") resolve(out);
      else reject(new Error("FileReader did not yield a string result"));
    };
    reader.readAsDataURL(file);
  });
}

function randomId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return Math.random().toString(36).slice(2);
}

export default AttachmentDropzone;
