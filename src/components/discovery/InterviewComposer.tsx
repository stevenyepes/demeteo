import React, { useEffect, useMemo, useState } from 'react';
import { Paperclip, Send } from 'lucide-react';

import { getAgentModels, modelSupportsImages } from '../../lib/agentModels';
import { addDiscoveryAttachment, removeDiscoveryAttachment } from '../../lib/discovery';
import { noVisionNote } from '../../lib/newDiscovery';
import type { AttachedFile } from '../../lib/attachments';
import type { ConfigOptionValue } from '../../types';
import { AttachmentDropzone, type DirectAttachmentPort } from '../AttachmentDropzone';
import { FieldLabel } from '../ui/FieldLabel';

interface InterviewComposerProps {
  discoveryId: string;
  /** The interviewer's own routing, which is what decides whether an image
   *  attached here will be read or only named. */
  agentKind: string;
  model: string;
  machineId: string;
  /** What the Discovery already holds. The list is the backend's, not this
   *  component's, which is what lets the chips stand across a remount. */
  attachments: AttachedFile[];
  /** An open question exists and nothing is streaming. */
  awaiting: boolean;
  pending: boolean;
  disabled: boolean;
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  /** Re-read the Discovery after a file is added or dropped. */
  onAttachmentsChanged: () => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
}

/**
 * Where a turn is typed.
 *
 * The placeholder flips with the interview's state because the two states ask
 * for different things: one settles the question above, the other opens a new
 * thread. Neither is the lesser — `DISCOVERY_UI_SPEC.md` §6.7 keeps the hint
 * saying that both settle the same question.
 *
 * **A file is attached before the turn that talks about it, never with one.**
 * `discovery_send_turn` carries text alone: attachments belong to the
 * Discovery, so the chip row survives the turn that added it and every later
 * turn is prompted with the same set (§3.4.6, PRD §4.6).
 *
 * **The vision note is here as well as on the New Discovery modal.** §3.4.6
 * gives this row only a paperclip and its chips, which left an image dropped
 * mid-interview into a model that cannot read one degrading in silence — and
 * §9.4's rule is that the UI does not say a thing it cannot do. The note is
 * soft either way: the file still rides, the agent is still told its path, and
 * only the inlining is lost.
 */
export function InterviewComposer({
  discoveryId,
  agentKind,
  model,
  machineId,
  attachments,
  awaiting,
  pending,
  disabled,
  value,
  onChange,
  onSend,
  onAttachmentsChanged,
  inputRef,
}: InterviewComposerProps): React.ReactElement {
  const [attachOpen, setAttachOpen] = useState(false);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [models, setModels] = useState<ConfigOptionValue[]>([]);
  const blocked = disabled || pending;

  // Probe-aware rather than name-only, as the New Discovery modal is: the
  // backend's own `supports_images` answers where it has one, and the name
  // heuristic is reached only for a model it does not carry.
  useEffect(() => {
    let cancelled = false;
    getAgentModels(machineId, agentKind)
      .then((list) => {
        if (!cancelled) setModels(list ?? []);
      })
      .catch(() => {
        if (!cancelled) setModels([]);
      });
    return () => {
      cancelled = true;
    };
  }, [machineId, agentKind]);

  const noVision = noVisionNote({
    model,
    readsImages: modelSupportsImages(models, agentKind, model),
    attachments,
  });

  const port: DirectAttachmentPort = useMemo(
    () => ({
      attachments,
      add: (input) => addDiscoveryAttachment(discoveryId, input),
      remove: (attachmentId) => removeDiscoveryAttachment(discoveryId, attachmentId),
    }),
    [discoveryId, attachments],
  );

  return (
    <div className="shrink-0 border-t border-white/5 bg-[#0d0f14]/90 px-4 py-3.5">
      {awaiting && (
        <p className="m-0 mb-2 text-[11px] text-slate-500">
          Pick an option above, or answer here — both settle the same question.
        </p>
      )}
      <div className="flex items-end gap-2.5">
        <input
          ref={inputRef}
          type="text"
          data-testid="interview-composer"
          aria-label="Answer the interviewer"
          value={value}
          disabled={blocked}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter') return;
            event.preventDefault();
            onSend();
          }}
          placeholder={awaiting ? 'Answer in your own words...' : 'Ask, answer, or push back...'}
          className="chat-input disabled:opacity-50"
        />
        <button
          type="button"
          data-testid="interview-attach"
          title="Attach a file or image"
          aria-label="Attach a file or image"
          aria-expanded={attachOpen}
          disabled={disabled}
          onClick={() => setAttachOpen((open) => !open)}
          className="btn-secondary disabled:cursor-not-allowed disabled:opacity-35"
        >
          <Paperclip className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
        <button
          type="button"
          data-testid="interview-send"
          aria-label="Send this turn"
          disabled={blocked || value.trim().length === 0}
          onClick={onSend}
          className="btn-primary disabled:cursor-not-allowed disabled:opacity-35"
        >
          <Send className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </div>

      {(attachOpen || attachments.length > 0) && (
        <div className="mt-2 flex flex-col gap-1.5">
          <FieldLabel className="mb-0">Attachments</FieldLabel>
          <AttachmentDropzone
            mode="direct"
            compact={!attachOpen}
            port={port}
            label="Attach"
            onError={setAttachmentError}
            onAdded={() => {
              setAttachmentError(null);
              onAttachmentsChanged();
            }}
            onRemoved={onAttachmentsChanged}
          />
          {attachmentError && (
            <p role="alert" className="font-mono text-[11px] text-ruby-200">
              {attachmentError}
            </p>
          )}
          {noVision && (
            <p
              data-testid="interview-no-vision"
              className="m-0 rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed text-amber-200/90"
            >
              {noVision.model} cannot read images.{' '}
              <span className="font-mono">{noVision.filenames.join(', ')}</span> will be attached
              and ignored.
            </p>
          )}
        </div>
      )}
    </div>
  );
}

export default InterviewComposer;
