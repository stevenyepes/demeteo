import React from "react";
import { X, Image as ImageIcon, FileText } from "lucide-react";
import type { AttachedFile } from "../lib/attachments";

interface AttachmentChipProps {
  attachment: AttachedFile;
  /** Optional local data URL for inline image previews — produced
   *  from the user's freshly-picked browser `File`. Null when the
   *  chip is for a post-launch attachment (no File handle). */
  previewUrl?: string | null;
  /** Remove handler. Omit for read-only chips (e.g. FeatureDetail). */
  onRemove?: (id: string) => void;
  /** Whole-chip click handler — used by FeatureDetail to open a viewer. */
  onClick?: (id: string) => void;
  /** Optional compact variant for the collapsed chip row. */
  compact?: boolean;
}

/**
 * One attachment chip — glass surface, image thumbnail when a
 * previewUrl is available and the mime is image/*, file-icon glyph
 * fallback otherwise. Filename + size are stacked under the
 * thumbnail; mime is rendered as a pill on the right.
 *
 * Hover surfaces a soft glow; focus surfaces a cyan outline. The
 * remove `x` is shown only when `onRemove` is supplied (read-only
 * chips render no remove affordance).
 */
export const AttachmentChip: React.FC<AttachmentChipProps> = ({
  attachment,
  previewUrl,
  onRemove,
  onClick,
  compact,
}) => {
  const isImage = attachment.mime.startsWith("image/");
  const showThumb = isImage && previewUrl;

  const handleRemove = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (onRemove) onRemove(attachment.id);
  };

  const interactive = Boolean(onClick);

  return (
    <div
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      onClick={() => interactive && onClick?.(attachment.id)}
      onKeyDown={(e) => {
        if (interactive && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onClick?.(attachment.id);
        }
      }}
      title={`${attachment.source_filename} · ${attachment.mime} · ${formatBytes(attachment.size)}`}
      className={[
        "group relative inline-flex items-center gap-2 rounded-lg border transition-all select-none",
        "bg-[rgba(18,22,30,0.75)] backdrop-blur-md",
        interactive
          ? "border-white/10 hover:border-cyan-400/40 hover:bg-[rgba(18,22,30,0.85)] cursor-pointer"
          : "border-white/10",
        compact ? "pr-2 pl-1 py-0.5" : "pr-2 pl-1.5 py-1",
      ].join(" ")}
    >
      <div
        className={[
          "shrink-0 rounded-md overflow-hidden flex items-center justify-center",
          "bg-black/40 border border-white/5",
          compact ? "w-6 h-6" : "w-9 h-9",
        ].join(" ")}
      >
        {showThumb ? (
          <img
            src={previewUrl!}
            alt={attachment.source_filename}
            className="w-full h-full object-cover"
            draggable={false}
          />
        ) : isImage ? (
          <ImageIcon className="w-4 h-4 text-cyan-300/80" />
        ) : (
          <FileText className="w-4 h-4 text-slate-400" />
        )}
      </div>

      <div className="min-w-0 flex flex-col leading-tight">
        <span
          className={[
            "truncate font-medium text-slate-100",
            compact ? "max-w-[120px] text-[11px]" : "max-w-[180px] text-xs",
          ].join(" ")}
        >
          {attachment.source_filename}
        </span>
        {!compact && (
          <span className="text-[10px] font-mono text-slate-500">
            {formatBytes(attachment.size)}
          </span>
        )}
      </div>

      <span
        className={[
          "shrink-0 font-mono uppercase tracking-wider text-[9px] px-1.5 py-0.5 rounded-md border",
          "border-violet-500/30 bg-violet-500/10 text-violet-300",
        ].join(" ")}
      >
        {shortMime(attachment.mime)}
      </span>

      {onRemove && (
        <button
          type="button"
          onClick={handleRemove}
          aria-label={`Remove ${attachment.source_filename}`}
          className={[
            "shrink-0 rounded-md text-slate-500 hover:text-ruby-300 hover:bg-ruby-500/10 transition-colors",
            compact ? "p-0.5" : "p-1",
          ].join(" ")}
        >
          <X className={compact ? "w-3 h-3" : "w-3.5 h-3.5"} />
        </button>
      )}
    </div>
  );
};

/**
 * Format bytes as a short human-readable label. Mirrors the
 * `formatBytes` logic used in chip rows elsewhere; kept local so
 * AttachmentChip remains self-contained.
 */
function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0 B";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/**
 * Compress a mime to a 4-char pill (`PNG`, `JPEG`, `PDF`, `TXT`, `JSON`,
 * `OCTET` …) for the chip's right-edge badge.
 */
function shortMime(mime: string): string {
  const primary = mime.split("/")[1] ?? mime;
  const cleaned = primary.split(";")[0].trim();
  return (cleaned || "BIN").slice(0, 7).toUpperCase();
}

export default AttachmentChip;
