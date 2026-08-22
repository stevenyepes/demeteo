import React, { useMemo, useState } from 'react';
import { EyeOff } from 'lucide-react';

import { addTicketAttachment, removeTicketAttachment } from '../../lib/discovery';
import { noVisionNote } from '../../lib/newDiscovery';
import type { AttachedFile } from '../../lib/attachments';
import { stagedCount } from '../../lib/ticketEditor';
import { AttachmentDropzone, type DirectAttachmentPort } from '../AttachmentDropzone';
import { Chip } from '../ui/Chip';
import { FieldLabel } from '../ui/FieldLabel';

interface TicketAttachmentsCardProps {
  ticketId: string;
  attachments: AttachedFile[];
  /** The model this ticket routes to, and whether it reads images — probed
   *  where the caller holds the model list, name-matched where it does not. */
  model: string;
  readsImages: boolean;
  /** Re-read the board: an attachment is written server-side the moment it is
   *  dropped, and the briefing above quotes the file names. */
  onChanged: () => void;
  disabled: boolean;
}

/**
 * Card 2 — `Attachments` (`DISCOVERY_UI_SPEC.md` §5.5, PRD §9.3).
 *
 * The launch dropzone, unmodified, against the Ticket instead of a Feature:
 * `ticket_add_attachment` is the same act `feature_add_attachment` is, one
 * aggregate over, which is exactly what `DirectAttachmentPort` exists for. A
 * ticket that is never started never writes an attachment row.
 *
 * The vision warning is soft and never withholds the file. The model is per
 * ticket (§9.3), so a plan that routes a screenshot-bearing ticket to a model
 * that cannot read one says so *where the model is chosen* rather than after
 * the run — and the file still rides, as a path the agent is told about.
 */
export function TicketAttachmentsCard({
  ticketId,
  attachments,
  model,
  readsImages,
  onChanged,
  disabled,
}: TicketAttachmentsCardProps): React.ReactElement {
  const [error, setError] = useState<string | null>(null);
  const [dismissedFor, setDismissedFor] = useState<string | null>(null);

  const port: DirectAttachmentPort = useMemo(
    () => ({
      attachments,
      add: (input) => addTicketAttachment(ticketId, input),
      remove: (attachmentId) => removeTicketAttachment(ticketId, attachmentId),
    }),
    [ticketId, attachments],
  );

  const note = noVisionNote({ model, readsImages, attachments });
  // Keyed on the model, so changing it resets the dismissal: the warning is
  // about *this* model, and a dismissal carried across would hide a new and
  // true statement behind an answer given to a different one.
  const dismissed = dismissedFor === model;

  return (
    <div className="nested-card flex flex-col gap-3.5 px-4 py-3.5">
      <div className="flex items-center justify-between gap-3">
        <FieldLabel className="mb-0">Attachments</FieldLabel>
        <Chip size="sm" tone="slate">
          {stagedCount(attachments.length)}
        </Chip>
      </div>

      {disabled ? (
        // A locked ticket's attachments went to its Feature when it started,
        // so there is nothing here to add to or remove from — the dropzone
        // would be a control writing to an aggregate that has moved on.
        <div className="flex flex-col gap-1">
          {attachments.length === 0 ? (
            <p className="m-0 text-[11px] text-slate-500">No attachments were staged.</p>
          ) : (
            attachments.map((file) => (
              <span key={file.id} className="font-mono text-[11px] break-all text-slate-400">
                {file.name}
              </span>
            ))
          )}
        </div>
      ) : (
        <AttachmentDropzone
          mode="direct"
          port={port}
          label="Add files"
          onError={setError}
          onAdded={() => {
            setError(null);
            onChanged();
          }}
          onRemoved={onChanged}
        />
      )}

      {error && (
        <p role="alert" className="m-0 font-mono text-[11px] text-ruby-200">
          {error}
        </p>
      )}

      {note && !dismissed && (
        <div
          data-testid="ticket-no-vision"
          className="flex items-start gap-2 rounded-lg border border-violet-500/40 bg-ruby-500/10 px-3 py-2 text-[11px] leading-relaxed text-ruby-200"
        >
          <EyeOff className="mt-0.5 h-3.5 w-3.5 shrink-0 text-ruby-300" aria-hidden="true" />
          <span className="min-w-0 flex-1">
            Model {note.model} does not read images — attachments will be referenced as paths only
            and not inlined.
          </span>
          <button
            type="button"
            title="Dismiss"
            aria-label="Dismiss"
            onClick={() => setDismissedFor(model)}
            className="shrink-0 text-ruby-300 transition hover:text-ruby-200"
          >
            ×
          </button>
        </div>
      )}

      <p className="m-0 text-[11px] leading-relaxed text-slate-500">
        {disabled
          ? 'This ticket has a feature. Its attachments were committed to it when it started.'
          : "A ticket has no feature to attach to yet, so these stage here and are committed the moment it starts — the same path a launch takes. The interview's own attachments stay with the interview."}
      </p>
    </div>
  );
}

export default TicketAttachmentsCard;
