import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { UploadCloud, FileWarning, FilePlus2 } from "lucide-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  addAttachment,
  extractClipboardImageFiles,
  recoverClipboardImageFile,
  stageBrowserFilesForLaunch,
  stageAttachmentMetadata,
  type AttachedFile,
  type AttachmentInput,
  type LaunchStageEntry,
} from "../lib/attachments";
import { formatError } from "../lib/errors";
import { AttachmentChip } from "./AttachmentChip";

/**
 * A staged attachment kept in local memory until the parent commits it
 * to a feature via `feature_add_attachment`. The Rust handler requires
 * a `feature_id` that only exists once a feature has been launched,
 * so pre-launch composers stage files here and resolve them after
 * the launch call returns.
 */
export type { LaunchStageEntry } from "../lib/attachments";

/**
 * `direct` mode against an aggregate that is not a Feature: the two commands
 * to call, and the list its owner already holds.
 *
 * Without one, `direct` writes through the Feature commands keyed by
 * `featureId` and shows only what it added this session. With one, the chips
 * are the owner's own list, so they stand across a remount — which is what a
 * composer that outlives the turn that added a file needs
 * (`DISCOVERY_UI_SPEC.md` §3.4.6).
 */
export interface DirectAttachmentPort {
  attachments: AttachedFile[];
  add: (input: AttachmentInput) => Promise<AttachedFile>;
  remove: (attachmentId: string) => Promise<void>;
}

interface AttachmentDropzoneProps {
  /** `launch` keeps entries local until {@link onCommitLaunch} runs;
   *  `direct` calls `addAttachment` immediately per pick. */
  mode: "launch" | "direct";
  /** Required for `direct` mode (the feature the attachment is added to),
   *  unless {@link AttachmentDropzoneProps.port} names another owner. */
  featureId?: string;
  /** Where `direct` mode writes, when it is not a Feature. */
  port?: DirectAttachmentPort;
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
 */
export const AttachmentDropzone: React.FC<AttachmentDropzoneProps> = ({
  mode,
  featureId,
  port,
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
  // When the user dismisses the native file dialog with Cancel,
  // browsers do NOT fire `change`, so the `isPicking` latch would stay
  // closed forever and the "Add files" button would silently no-op on
  // the next click. Listen to the window `focus` event while a picker
  // is open: when the OS dialog steals focus and then gives it back,
  // a `change` fires (with files) for a selection or doesn't fire for a
  // cancel. We can't observe the cancel directly — but if focus has
  // returned and the input still has no files, the user cancelled, so
  // release the latch.
  const openPicker = useCallback(() => {
    if (isPicking) return;
    setIsPicking(true);
    inputRef.current?.click();
  }, [isPicking]);

  useEffect(() => {
    if (!isPicking) return;
    const onFocus = () => {
      // Defer one tick so `onChange` (which is queued first by the
      // browser when files were selected) can populate `inputRef.current.files`
      // before we check it.
      window.setTimeout(() => {
        const hasFiles =
          inputRef.current?.files && inputRef.current.files.length > 0;
        if (!hasFiles) {
          setIsPicking(false);
        }
      }, 0);
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [isPicking]);

  const onPickerChange = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      setIsPicking(false);
      const files = e.target.files;
      if (!files || files.length === 0) return;
      // Validate client-side before ingesting — the OS picker lets users
      // switch to "All Files" and select anything, even though the
      // `accept` attribute is a hint only. Surface a soft error per
      // rejected file so the user knows what was dropped.
      const allowed: File[] = [];
      for (const f of Array.from(files)) {
        if (isAllowedFile(f)) {
          allowed.push(f);
        } else {
          onError?.(
            `File not allowed: ${f.name} — supported types are png, jpg, gif, webp, pdf, txt, md, json.`,
          );
        }
      }
      if (allowed.length > 0) {
        await ingestFiles(allowed);
      }
      // Reset so the same file can be re-picked after a remove.
      e.target.value = "";
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mode, featureId, stageEntries, onError],
  );

  // -- shared ingest ------------------------------------------------------
  const ingestFiles = useCallback(
    async (files: File[]) => {
      if (mode === "launch") {
        if (!onChangeStage) {
          throw new Error("AttachmentDropzone: onChangeStage is required for launch mode");
        }
        try {
          onChangeStage(await stageBrowserFilesForLaunch(files, stageEntries ?? []));
        } catch (err) {
          onError?.(formatError(err));
        }
        return;
      }

      for (const file of files) {
        try {
          await ingestOneDirect({ kind: "file", file });
        } catch (err) {
          const message = formatError(err);
          onError?.(message);
        }
      }
    },
    // `ingestOneDirect` is declared below with the other direct-mode helpers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mode, featureId, stageEntries, onChangeStage, onError],
  );

  const handlePaste = useCallback(
    async (e: React.ClipboardEvent<HTMLDivElement>) => {
      const extraction = extractClipboardImageFiles(e.clipboardData);
      if (extraction.kind === "none") {
        // WebKitGTK can send an empty item list for an image paste. Do not
        // probe ordinary text/unsupported pastes, which still expose items.
        if (e.clipboardData.items.length !== 0) return;
        const recovery = await recoverClipboardImageFile();
        if (recovery.kind !== "recovered") {
          onError?.(
            "This webview could not read image bytes from the clipboard. Save it and attach it, or try another clipboard source.",
          );
          return;
        }
        e.preventDefault();
        await ingestFiles([recovery.file]);
        return;
      }
      if (extraction.kind === "unavailable") {
        onError?.(
          "The clipboard offered an image, but this webview could not access its file. Save it and attach it, or try another clipboard source.",
        );
        return;
      }
      e.preventDefault();
      await ingestFiles(extraction.files);
    },
    [ingestFiles, onError],
  );

  const ingestPaths = useCallback(
    async (paths: string[]) => {
      for (const sourcePath of paths) {
        const lower = sourcePath.toLowerCase();
        const sourceFilename = sourcePath.split(/[\\/]/).pop() ?? sourcePath;
        if (!isAllowedPath(sourcePath)) {
          onError?.(
            `File not allowed: ${sourceFilename} — supported types are png, jpg, gif, webp, pdf, txt, md, json.`,
          );
          continue;
        }
        const mime = guessMime(lower);
        try {
          if (mode === "direct") {
            await ingestOneDirect({ kind: "path", sourcePath, sourceFilename, mime });
          } else {
            await ingestOneLaunch({ kind: "path", sourcePath, sourceFilename, mime });
          }
        } catch (err) {
          const message = formatError(err);
          onError?.(message);
        }
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mode, featureId, stageEntries, onError],
  );

  const ingestOneDirect = useCallback(
    async (input: AttachmentInput) => {
      if (port) {
        onAdded?.(await port.add(input));
        return;
      }
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
    [featureId, port, onAdded],
  );

  const ingestOneLaunch = useCallback(
    async (input: Extract<AttachmentInput, { kind: "path" }>) => {
      if (!onChangeStage) {
        throw new Error(
          "AttachmentDropzone: onChangeStage is required for launch mode",
        );
      }
      const sourceFilename =
        input.sourceFilename ?? input.sourcePath.split(/[\\/]/).pop() ?? "attachment";
      const mime = input.mime ?? guessMime(sourceFilename.toLowerCase());
      const sourcePath = input.sourcePath;

      // Compute `sha256` + `size` from whatever bytes the ingest
      // surfaced. File-based picks get Web Crypto over the browser
      // `File` (already in memory). Path-based picks (Tauri drag-
      // and-drop yields only the absolute path, no browser `File`)
      // MUST go through a Rust IPC to read the bytes — without it
      // the chip would show "0 B" and two drops of the same path
      // would produce two chips (the dedup filter compares
      // `sha256`, so a fresh "staged-<uuid>" key survives both
      // drops — see the bug repro tests/repro/attachment-dnd-staging.mjs).
      let sha256: string;
      let size: number;
      try {
        const meta = await stageAttachmentMetadata(
          input.sourcePath,
          input.mime ?? null,
          sourceFilename,
        );
        sha256 = meta.sha256;
        size = meta.size;
      } catch (err) {
        const message = formatError(err);
        onError?.(message);
        return;
      }

      const entry: LaunchStageEntry = {
        sha256,
        name: sourceFilename,
        source_filename: sourceFilename,
        mime,
        size,
        previewUrl: null,
        file: null,
        sourcePath,
      };

      onChangeStage([...(stageEntries ?? []).filter((e) => e.sha256 !== sha256), entry]);
    },
    [onChangeStage, stageEntries, onError],
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
      if (port) {
        const owner = port;
        return owner.attachments.map((a) => ({
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
            try {
              await owner.remove(a.id);
              onRemoved?.(a.id);
            } catch (err) {
              onError?.(formatError(err));
            }
          },
        }));
      }
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
            const message = formatError(err);
            onError?.(message);
          }
        },
      }));
    }
    return [];
  }, [mode, stageEntries, directAttachments, featureId, port, onChangeStage, onRemoved, onError]);

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
      tabIndex={0}
      onPaste={handlePaste}
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
            or drop here · click to pick · paste an image · png / jpg / webp / gif / pdf / txt · max 100 MB each · 10 per feature
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

/**
 * Lowercase extension set mirroring the Rust-side allow-list in
 * `domain::attachment::mime_for_ext`. The `accept` attribute on the
 * file input is a hint only — the user can switch the picker to "All
 * Files" and select anything — so we re-check here and surface a soft
 * error per rejected file. Keep this list in sync with the Rust
 * validation in `commands::attachments::feature_add_attachment`.
 */
const ACCEPTED_EXTS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "pdf",
  "txt",
  "md",
  "markdown",
  "json",
]);

const SUPPORTED_IMAGE_MIMES: ReadonlySet<string> = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
]);

function isAllowedFile(file: File): boolean {
  const mime = (file.type || "").toLowerCase();
  if (SUPPORTED_IMAGE_MIMES.has(mime)) return true;
  const name = file.name.toLowerCase();
  const dot = name.lastIndexOf(".");
  if (dot < 0) return false;
  const ext = name.slice(dot + 1);
  if (ACCEPTED_EXTS.has(ext)) return true;
  // Fall back to the browser-provided mime when the extension is
  // ambiguous (e.g. ".bash_history" has no extension at all but
  // reports text/plain). The Rust side mirrors this check against
  // `mime_for_ext`.
  if (mime === "application/pdf") return true;
  if (mime === "text/plain" || mime === "text/markdown" || mime === "application/json") {
    return true;
  }
  return false;
}

/**
 * Drag-and-drop variant of {@link isAllowedFile}. The Tauri webview
 * hands us absolute paths only (no mime), so the check is purely
 * extension-based and mirrors `isAllowedFile`'s positive list.
 */
function isAllowedPath(sourcePath: string): boolean {
  const lower = sourcePath.toLowerCase();
  const slash = Math.max(lower.lastIndexOf("/"), lower.lastIndexOf("\\"));
  const tail = slash >= 0 ? lower.slice(slash + 1) : lower;
  const dot = tail.lastIndexOf(".");
  if (dot < 0) return false;
  return ACCEPTED_EXTS.has(tail.slice(dot + 1));
}

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

export default AttachmentDropzone;
