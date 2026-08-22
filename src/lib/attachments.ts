import { invoke } from "@tauri-apps/api/core";

/**
 * Typed IPC wrappers for the per-feature attachment subsystem.
 *
 * Mirrors the shape of `src/lib/agentModels.ts` — never call `invoke()`
 * directly from a component (`AGENTS.md` §4). All commands correspond to
 * the `feature_add_attachment` / `feature_list_attachments` /
 * `feature_remove_attachment` / `attachment_read` Tauri handlers in
 * `src-tauri/src/commands/attachments.rs`.
 *
 * **Wire contract.** The Rust side stores attachments as content-addressable
 * blobs under `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>`
 * (see `domain::attachment::AttachedFile`). Re-uploading the same bytes
 * under a different filename is idempotent — the manifest row is updated
 * to reflect the new name but the on-disk file is shared.
 *
 * **Launch-stage model.** Because `feature_add_attachment` requires a
 * `feature_id` (the Rust side has no "staged / null feature" code path —
 * the spec's stage-then-claim model collapsed into per-feature attach in
 * sub-1), pre-launch composers ({@link StartFeatureModal},
 * {@link ProjectHome}) collect File handles + paths locally in a
 * `LaunchStaging` Map and call `addAttachment` only after the launching
 * feature id is known. This decouples UI feedback from feature creation
 * and keeps the chip list live during form fill-in.
 *
 * **Preview reads.** Post-launch, attachments fetched via
 * {@link listAttachments} arrive without a browser `File` handle (they
 * came in through Tauri drag-and-drop, which yields absolute paths
 * only). {@link readAttachment} round-trips the bytes back to the
 * webview through the `attachment_read` IPC so the preview Modal can
 * render out-of-session files. {@link getAttachmentDataUrl} is the
 * thin shim that turns those bytes into a `data:` URL — when a
 * browser `File` is available it goes through FileReader (instant,
 * no IPC); when only the manifest row is available it falls through
 * to {@link readAttachment} provided a `featureId` is supplied.
 */
export interface AttachedFile {
  /** Backend-generated stable id, format `at-<random>`. */
  id: string;
  /** Sanitized display name (used for chip labels). */
  name: string;
  /** IANA mime, e.g. `image/png`. */
  mime: string;
  /** Lowercase hex SHA-256 of the on-disk bytes. */
  sha256: string;
  size: number;
  /** Original user-supplied filename, preserved verbatim for the UI. */
  source_filename: string;
}

/**
 * A browser-file attachment retained until a newly launched Feature has an
 * id. This is intentionally frontend-only: callers persist these through the
 * existing `feature_add_attachment` flow after launch.
 */
export interface LaunchStageEntry {
  sha256: string;
  name: string;
  source_filename: string;
  mime: string;
  size: number;
  previewUrl: string | null;
  file: File | null;
  sourcePath: string | null;
}

/**
 * Input accepted by {@link addAttachment}. Either a browser `File`
 * (from `<input type="file">` or clipboard paste) OR an absolute local
 * path string (from Tauri's drag-drop event or `plugin-dialog`).
 *
 * The wrapper never reads bytes through this object — the Rust command
 * reads the file directly from disk via `std::fs::read`. Local `File`
 * handles are kept only so the UI can render a preview thumbnail.
 */
export type AttachmentInput =
  | { kind: "file"; file: File }
  | { kind: "path"; sourcePath: string; sourceFilename?: string; mime?: string };

/**
 * Add an attachment to a feature.
 *
 * Routes to the Tauri command `feature_add_attachment`. The Rust side
 * validates size (max 100 MiB), dedupes by content hash, and writes
 * the bytes under `<attachments_root>/<feature_id>/<sha256>.<ext>`.
 *
 * **Bytes vs path.** Modern Chromium / Tauri 2 webviews strip the
 * legacy `File.path` extension on `<input type="file">` selections
 * for security, so a click-picked browser `File` typically arrives
 * without an absolute disk path. When the input is a `file` kind
 * (drag-and-drop's "path" kind still has an absolute path), we read
 * the bytes into memory and ferry them through IPC as a JSON number
 * array — the Rust command accepts either a path or bytes. Drag-and-
 * drop paths continue to take the path branch unchanged.
 *
 * @param featureId target feature id (already-launched features only).
 * @param input     file handle (with preview-friendly FileReader) or
 *                  absolute path (drag-and-drop / native dialog).
 */
export async function addAttachment(
  featureId: string,
  input: AttachmentInput,
): Promise<AttachedFile> {
  const wire = await attachmentWire(input);
  return invoke<AttachedFile>("feature_add_attachment", {
    featureId,
    sourcePath: wire.sourcePath,
    mime: wire.mime,
    sourceFilename: wire.sourceFilename,
    bytes: wire.bytes,
  });
}

/** The four arguments every `*_add_attachment` command takes, whichever way
 *  the file was picked. Shared so a second owner (a Ticket, a Discovery)
 *  cannot decide the bytes-vs-path question differently from a Feature. */
export interface AttachmentWire {
  sourcePath: string;
  mime: string | null;
  sourceFilename: string | null;
  bytes: number[] | null;
}

/**
 * Normalise a pick into that shape.
 *
 * The browser gives a `File` but no usable absolute path (the common case in
 * Tauri 2 — `file.path` is stripped), so its bytes are read here and ferried
 * through IPC. Drag-and-drop yields a path and keeps the path branch, where
 * the Rust command reads the bytes itself.
 */
export async function attachmentWire(input: AttachmentInput): Promise<AttachmentWire> {
  if (input.kind === "file") {
    const bytes = new Uint8Array(await input.file.arrayBuffer());
    return {
      sourcePath: "",
      mime: input.file.type || null,
      sourceFilename: input.file.name,
      bytes: Array.from(bytes),
    };
  }
  return {
    sourcePath: input.sourcePath,
    mime: input.mime ?? null,
    sourceFilename: input.sourceFilename ?? pathBasename(input.sourcePath),
    bytes: null,
  };
}

/**
 * List every attachment on a feature. Returns `[]` when the feature has
 * no attachments column populated (the manifest column default).
 */
export async function listAttachments(featureId: string): Promise<AttachedFile[]> {
  const list = await invoke<AttachedFile[]>("feature_list_attachments", {
    featureId,
  });
  return Array.isArray(list) ? list : [];
}

/**
 * Remove an attachment. Idempotent: the Rust side is a no-op if the id
 * is already gone. The on-disk file is shared by content hash, so it
 * is deleted only when no other manifest row references the same sha256.
 */
export async function removeAttachment(
  featureId: string,
  attachmentId: string,
): Promise<void> {
  await invoke<void>("feature_remove_attachment", {
    featureId,
    attachmentId,
  });
}

/**
 * Result of {@link readAttachment}: the on-disk mime plus the raw bytes.
 *
 * The Rust command serializes `Vec<u8>` as a JSON array of numbers
 * (0-255), which we repack into a `Uint8Array` on the JS side. For
 * preview-only display paths (e.g. an `<img src="data:...">` Modal).
 *
 * Never used on the prompt-injection path — the orchestrator mirrors
 * bytes into the per-step worktree on the Rust side, not via IPC.
 */
export interface AttachmentBytes {
  mime: string;
  bytes: Uint8Array;
}

/**
 * Fetch the bytes of a previously-uploaded attachment via the
 * `attachment_read` IPC.
 *
 * Use case: a preview Modal needs to render an out-of-session file
 * (one that arrived through Tauri drag-and-drop with no browser
 * `File` handle). Resolves the manifest row server-side, so callers
 * don't need to pass `mime` or `sha256` — they get both back.
 *
 * Throws when the feature or attachment id is not present in the
 * manifest, when the feature is missing on disk, or when the
 * underlying bytes can't be read.
 */
export async function readAttachment(
  featureId: string,
  attachmentId: string,
): Promise<AttachmentBytes> {
  const raw = await invoke<number[]>("attachment_read", {
    featureId,
    attachmentId,
  });
  const manifest = await listAttachments(featureId);
  const meta = manifest.find((a) => a.id === attachmentId);
  return {
    mime: meta?.mime ?? "application/octet-stream",
    bytes: Uint8Array.from(raw),
  };
}

/**
 * Generate a `data:<mime>;base64,…` URL for a picked file, or null
 * when no source can produce bytes.
 *
 * Used by the chip preview + hover-preview surfacing in
 * {@link AttachmentChip} / {@link AttachmentDropzone}. Three modes:
 *
 * 1. A browser `File` is supplied → FileReader path (instant, no IPC).
 * 2. A `featureId` is supplied → falls through to
 *    {@link readAttachment} via the `attachment_read` IPC. This is the
 *    post-launch preview path for files that came in through Tauri
 *    drag-and-drop (no browser File handle).
 * 3. Neither → returns null. Pre-launch callers that haven't supplied
 *    a File yet fall back to a mime-icon chip; see {@link AttachmentChip}.
 */
export async function getAttachmentDataUrl(
  attachment: AttachedFile,
  file?: File | null,
  featureId?: string,
): Promise<string | null> {
  if (file) {
    return readFileAsDataUrl(file);
  }
  if (featureId) {
    const { mime, bytes } = await readAttachment(featureId, attachment.id);
    return bytesToDataUrl(mime, bytes);
  }
  return null;
}

function bytesToDataUrl(mime: string, bytes: Uint8Array): string {
  // Chunked String.fromCharCode avoids a "Maximum call stack size
  // exceeded" on the larger file cap (25 MiB) where a single
  // fromCharCode(bytes) would blow the JS argument limit.
  let binary = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const slice = bytes.subarray(i, Math.min(i + CHUNK, bytes.length));
    binary += String.fromCharCode.apply(null, Array.from(slice));
  }
  const b64 = btoa(binary);
  return `data:${mime};base64,${b64}`;
}

/**
 * Exact allow-list of clipboard image MIME types that the rest of the
 * pipeline accepts. Kept in sync with the Rust
 * `commit_attachment_inner` allow-list at
 * `crates/demeteo-core/src/application/attachments.rs:221-235`; a
 * clipboard item outside this set never reaches `addAttachment` and
 * the underlying `feature_add_attachment` would reject it anyway.
 */
const SUPPORTED_CLIPBOARD_IMAGE_MIMES: ReadonlySet<string> = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/tiff",
]);

/**
 * The browser-visible outcome of inspecting clipboard file entries for
 * supported images. `unavailable` means a supported image was advertised,
 * but its bytes cannot be obtained through the webview's `File` API.
 */
export type ClipboardImageExtraction =
  | { kind: "files"; files: File[] }
  | { kind: "none" }
  | { kind: "unavailable"; mime: string };

/**
 * Result of the async Clipboard API recovery path. This is deliberately
 * separate from {@link ClipboardImageExtraction}: callers use it only when a
 * paste event exposed no items at all (the WebKitGTK 218519 shape).
 */
export type AsyncClipboardImageRecovery =
  | { kind: "recovered"; file: File }
  | { kind: "unavailable" }
  | { kind: "denied" }
  | { kind: "no-bytes" };

/**
 * Read one supported clipboard image directly from the async Clipboard API.
 *
 * This must be called from the paste gesture, after the synchronous
 * DataTransfer path found no image. It does not broaden the MIME allow-list:
 * only types in {@link SUPPORTED_CLIPBOARD_IMAGE_MIMES} are requested.
 */
export async function recoverClipboardImageFile(): Promise<AsyncClipboardImageRecovery> {
  const clipboard = navigator.clipboard;
  if (!clipboard?.read) return { kind: "unavailable" };

  let items: ClipboardItem[];
  try {
    items = await clipboard.read();
  } catch {
    return { kind: "denied" };
  }

  for (const item of items) {
    for (const advertisedType of item.types) {
      const mime = advertisedType.toLowerCase();
      if (!SUPPORTED_CLIPBOARD_IMAGE_MIMES.has(mime)) continue;
      try {
        const blob = await item.getType(advertisedType);
        if (blob.size === 0) continue;
        const extension = mime === "image/jpeg" ? "jpg" : mime.slice("image/".length);
        return {
          kind: "recovered",
          file: new File([blob], `pasted-image.${extension}`, { type: mime }),
        };
      } catch {
        // Another clipboard item may still expose usable bytes.
      }
    }
  }

  return { kind: "no-bytes" };
}

/**
 * Extract supported clipboard images while preserving the distinction between
 * no supported image and a supported image the browser cannot expose as a
 * `File`. MIME matching and unavailable MIME reporting are lowercase.
 *
 * Text, HTML, and unsupported entries are deliberately not inspected: this
 * helper only invokes `getAsFile()` after an item passes the existing file
 * kind and MIME allow-list checks.
 */
export function extractClipboardImageFiles(
  clipboardData: DataTransfer,
): ClipboardImageExtraction {
  const files: File[] = [];
  const items = clipboardData.items;
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.kind !== "file") continue;

    const mime = item.type.toLowerCase();
    if (!SUPPORTED_CLIPBOARD_IMAGE_MIMES.has(mime)) continue;

    const file = item.getAsFile();
    if (file === null) return { kind: "unavailable", mime };
    files.push(file);
  }

  return files.length > 0 ? { kind: "files", files } : { kind: "none" };
}

/**
 * Extract the supported image `File` handles from a clipboard /
 * drag-paste `DataTransfer`. Returns the items in clipboard order;
 * `[]` when no supported image is present.
 *
 * Only entries whose `kind === "file"` and whose `type` matches the
 * exact allow-list of supported image MIME types (compared
 * case-insensitively) are considered. `getAsFile()` is invoked only
 * for items that pass both filters. A supported item whose file is
 * unavailable maps to an empty compatibility result; callers needing to
 * distinguish it from no image should use {@link extractClipboardImageFiles}.
 *
 * Pure: no I/O, no IPC, no `preventDefault`, no filename normalization
 * or hashing — those happen later in `ingestFiles` /
 * `feature_add_attachment`. The caller (e.g.
 * `AttachmentDropzone.handlePaste`) decides whether to swallow the
 * event based on whether this helper returned a non-empty list.
 */
export function extractImageFilesFromClipboard(
  clipboardData: DataTransfer,
): File[] {
  const extraction = extractClipboardImageFiles(clipboardData);
  return extraction.kind === "files" ? extraction.files : [];
}

/**
 * Compute SHA-256 hex over a browser `File` using `crypto.subtle`.
 * Used by the launch-stage dedup in {@link AttachmentDropzone} so the
 * staging Map keys the same way the Rust `feature_add_attachment`
 * command keys the manifest (sha256). Returns lowercase hex (64 chars).
 */
export async function computeLocalSha256(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const digest = await crypto.subtle.digest("SHA-256", buf);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/**
 * Build launch-stage entries for browser `File` values and replace prior
 * entries with matching byte hashes. All browser-file launch entry points
 * (picker, paste, modal, and composer) share this so their MIME fallback,
 * preview generation, and SHA-256 deduplication cannot drift.
 */
export async function stageBrowserFilesForLaunch(
  files: readonly File[],
  stageEntries: readonly LaunchStageEntry[],
): Promise<LaunchStageEntry[]> {
  let next = [...stageEntries];

  for (const file of files) {
    const sourceFilename = file.name;
    const mime = file.type.toLowerCase() || guessBrowserFileMime(sourceFilename.toLowerCase());
    const sha256 = await computeLocalSha256(file);
    const previewUrl = mime.startsWith("image/") ? await readFileAsDataUrl(file) : null;
    const entry: LaunchStageEntry = {
      sha256,
      name: sourceFilename,
      source_filename: sourceFilename,
      mime,
      size: file.size,
      previewUrl,
      file,
      sourcePath: null,
    };
    next = [...next.filter((existing) => existing.sha256 !== sha256), entry];
  }

  return next;
}

/**
 * One file collected before the aggregate that will own it exists — the wire
 * shape `start_feature`, `remote_submit_run` and `discovery_create` all take.
 */
export interface StagedAttachmentInput {
  source_path: string;
  mime: string | null;
  source_filename: string | null;
  bytes: number[] | null;
}

/**
 * Convert a staged batch into that shape.
 *
 * The whole batch travels with the create call rather than as a follow-up per
 * file, which is what stops the agent's first turn from racing the attachments
 * it is supposed to have been given.
 */
export async function stagedAttachmentInputs(
  entries: readonly LaunchStageEntry[],
): Promise<StagedAttachmentInput[]> {
  return Promise.all(
    entries.map(async (entry) => ({
      source_path: entry.sourcePath ?? "",
      mime: entry.mime ?? null,
      source_filename: entry.source_filename ?? null,
      bytes: entry.file ? Array.from(new Uint8Array(await entry.file.arrayBuffer())) : null,
    })),
  );
}

/**
 * Stage-time metadata for a path-based attachment pick (Tauri
 * drag-and-drop yields an absolute path; no browser `File` is
 * available to Web Crypto).
 *
 * Routes to the Tauri command `attachment_stage_metadata`. The Rust
 * side reads the bytes from `sourcePath`, sha256s them, and returns
 * the size — mirroring the bytes-fetch + sha256 surface that
 * `feature_add_attachment` produces, minus the feature-scoped
 * storage step (no `feature_id` exists at staging time). The React
 * launch-stage uses `sha256` as both the React key AND the
 * re-drop dedup signal, and `size` so the chip renders the real
 * byte count instead of a confusing "0 B".
 *
 * Errors propagate from Rust as `AppError::validation` (missing
 * file, oversize, unsupported mime/ext) so the dropzone surfaces
 * an inline error instead of a silently-staged entry.
 */
export interface StagedAttachmentMeta {
  /** Lowercase hex SHA-256 of the on-disk bytes (matches the
   *  `feature_add_attachment` server-side computation byte-for-byte). */
  sha256: string;
  /** Byte length, used to render the chip's size label. */
  size: number;
}

/**
 * Fetch the staging-time metadata for a dropped file.
 *
 * @param sourcePath absolute disk path from Tauri drag-and-drop.
 * @param mime       optional mime hint (frontend can infer from filename).
 * @param sourceFilename optional original filename (used to infer mime).
 */
export async function stageAttachmentMetadata(
  sourcePath: string,
  mime?: string | null,
  sourceFilename?: string | null,
): Promise<StagedAttachmentMeta> {
  return invoke<StagedAttachmentMeta>("attachment_stage_metadata", {
    sourcePath,
    mime: mime ?? null,
    sourceFilename: sourceFilename ?? null,
  });
}

function readFileAsDataUrl(file: File): Promise<string> {
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

function guessBrowserFileMime(lowerFilename: string): string {
  if (lowerFilename.endsWith(".png")) return "image/png";
  if (lowerFilename.endsWith(".jpg") || lowerFilename.endsWith(".jpeg")) return "image/jpeg";
  if (lowerFilename.endsWith(".gif")) return "image/gif";
  if (lowerFilename.endsWith(".webp")) return "image/webp";
  if (lowerFilename.endsWith(".tif") || lowerFilename.endsWith(".tiff")) return "image/tiff";
  if (lowerFilename.endsWith(".pdf")) return "application/pdf";
  if (lowerFilename.endsWith(".md") || lowerFilename.endsWith(".markdown")) return "text/markdown";
  if (lowerFilename.endsWith(".txt")) return "text/plain";
  if (lowerFilename.endsWith(".json")) return "application/json";
  return "application/octet-stream";
}

/**
 * Split a path on either separator and return the trailing segment.
 * Used by the drag-and-drop "path" branch where we already have an
 * absolute disk path and only need the filename for the manifest row.
 */
function pathBasename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts.length === 0 ? p : parts[parts.length - 1];
}
