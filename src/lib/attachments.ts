import { invoke } from "@tauri-apps/api/core";

/**
 * Typed IPC wrappers for the per-feature attachment subsystem.
 *
 * Mirrors the shape of `src/lib/agentModels.ts` — never call `invoke()`
 * directly from a component (`AGENTS.md` §4). All commands correspond to
 * the `feature_add_attachment` / `feature_list_attachments` /
 * `feature_remove_attachment` Tauri handlers in
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
 * **Preview reads.** Sub-1 does not yet expose an
 * `attachment_read`/`attachment_get_bytes` command, so
 * {@link getAttachmentDataUrl} generates a local data URL via FileReader
 * when a browser `File` handle is available. Post-launch attachments
 * fetched from {@link listAttachments} (no local File) return null and
 * the caller falls back to a mime-icon chip — see
 * {@link AttachmentChip}. The wrapper is forward-compatible: when an
 * IPC read command lands, only this function needs to change.
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
 * On a browser `File` input, the caller is responsible for keeping a
 * copy of the File handle — see {@link getAttachmentDataUrl} — since
 * the Tauri command consumes the path, not the in-memory blob.
 *
 * @param featureId target feature id (already-launched features only).
 * @param input     file handle (with preview-friendly FileReader) or
 *                  absolute path (drag-and-drop / native dialog).
 */
export async function addAttachment(
  featureId: string,
  input: AttachmentInput,
): Promise<AttachedFile> {
  const sourcePath = input.kind === "file" ? resolvePathFromFile(input.file) : input.sourcePath;
  if (!sourcePath) {
    throw new Error(
      "Cannot determine an absolute file path for this attachment — only browser File handles and drag-and-dropped paths are supported.",
    );
  }
  const sourceFilename =
    input.kind === "file" ? input.file.name : input.sourceFilename ?? pathBasename(input.sourcePath);
  const mime = input.kind === "file" ? (input.file.type || undefined) : input.mime;

  return invoke<AttachedFile>("feature_add_attachment", {
    featureId,
    sourcePath,
    mime: mime ?? null,
    sourceFilename: sourceFilename ?? null,
  });
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
 * Generate a `data:<mime>;base64,…` URL for a picked file, or return
 * null when no local `File` handle is available.
 *
 * Used by the chip preview + hover-preview surfacing in
 * {@link AttachmentChip} / {@link AttachmentDropzone}. For a file the
 * user just picked in the same session the local `File` is supplied and
 * the data URL is instant. For post-launch attachments (or files that
 * arrived through Tauri's drop protocol, which yields paths only), the
 * call returns null and the chip falls back to a mime-icon — the
 * underlying bytes are still on disk in
 * `<attachments_root>/<feature_id>/<sha256>.<ext>`, but reading them
 * requires a future `attachment_read` IPC.
 */
export async function getAttachmentDataUrl(
  _attachment: AttachedFile,
  file?: File | null,
): Promise<string | null> {
  if (!file) return null;
  return readFileAsDataUrl(file);
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

/**
 * The browser `File` type strips the absolute path (by spec, for
 * security). The only way to recover one is via the legacy
 * `webkitRelativePath` / `path` extension that Chromium exposes only
 * when the file was picked via `<input type="file">` — and even there
 * it's deprecated. Returns the relative filename as the safest
 * fallback so the backend command always has *some* source to read.
 */
function resolvePathFromFile(file: File): string {
  const legacy = file as File & { path?: string };
  if (typeof legacy.path === "string" && legacy.path.length > 0) {
    return legacy.path;
  }
  return file.name;
}

function pathBasename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts.length === 0 ? p : parts[parts.length - 1];
}
