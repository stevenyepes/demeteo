import React, { useMemo, useRef, useState } from 'react';
import { Info } from 'lucide-react';

import { nothingLeftToSettle, openQuestionKey } from '../../lib/discoveryInterview';
import type { TranscriptBlock } from '../../lib/discoveryInterview';
import { EFFORT_LABELS } from '../../lib/effortLevels';
import type { Discovery, DiscoveryMessageView, QuestionOption } from '../../types';
import { Chip } from '../ui/Chip';
import { ColumnSubHeader } from './ColumnSubHeader';
import { ConfinementBanner } from './ConfinementBanner';
import { InterviewComposer } from './InterviewComposer';
import { InterviewTranscript } from './InterviewTranscript';
import type { DiscoveryStreamStore } from './useDiscoveryStream';

interface InterviewColumnProps {
  discovery: Discovery;
  messages: DiscoveryMessageView[];
  /** Derived once in the workspace, which also reads the open question off
   *  them. */
  blocks: TranscriptBlock[];
  /** Resolved machine name, or the id while it is unknown. */
  machineLabel: string;
  pending: boolean;
  store: DiscoveryStreamStore;
  onSend: (text: string) => void;
  /** Re-read the Discovery — its attachments are what the composer edits. */
  onRefresh: () => void;
}

/**
 * The interview: what the interviewer may do, what it has said, and where the
 * next turn is typed (`DISCOVERY_UI_SPEC.md` §3.4).
 *
 * Picking an option and typing an answer take the same path out of here. The
 * option's label is what the next prompt carries, verbatim, which is the only
 * difference between the two (`docs/TASKS_DISCOVERY.md`, "The interview turn
 * contract").
 */
export function InterviewColumn({
  discovery,
  messages,
  blocks,
  machineLabel,
  pending,
  store,
  onSend,
  onRefresh,
}: InterviewColumnProps): React.ReactElement {
  const [draft, setDraft] = useState('');
  const composerRef = useRef<HTMLInputElement | null>(null);

  const openQuestion = useMemo(() => openQuestionKey(blocks), [blocks]);

  const closed = discovery.status === 'closed';
  const awaiting = openQuestion !== null && !pending;
  const exhausted = openQuestion === null && !pending && nothingLeftToSettle(messages);

  function send(text: string) {
    const trimmed = text.trim();
    if (trimmed.length === 0 || pending || closed) return;
    setDraft('');
    onSend(trimmed);
  }

  function pick(option: QuestionOption) {
    send(option.label);
  }

  return (
    <div className="flex w-[560px] min-h-0 shrink-0 flex-col border-r border-white/5 bg-[#0b0d12]/40">
      <ColumnSubHeader title="Interview">
        <Chip size="sm" tone="cyan">
          {discovery.agent_kind}
        </Chip>
        {discovery.model && (
          <Chip size="sm" tone="violet" maxWidth="10rem">
            {discovery.model}
          </Chip>
        )}
        {discovery.effort && (
          <Chip size="sm" tone="slate">
            effort {EFFORT_LABELS[discovery.effort]}
          </Chip>
        )}
        <Chip size="sm" tone="slate" maxWidth="8rem">
          {machineLabel}
        </Chip>
      </ColumnSubHeader>

      <ConfinementBanner agentKind={discovery.agent_kind} />

      <InterviewTranscript
        discoveryId={discovery.id}
        blocks={blocks}
        openQuestion={openQuestion}
        pending={pending}
        store={store}
        onPick={pick}
        onAnswerInOwnWords={() => composerRef.current?.focus()}
      />

      {exhausted && (
        <div
          data-testid="interview-advisory"
          className="mx-4 mb-2.5 flex shrink-0 items-center gap-2 rounded-lg border border-white/5 bg-white/[0.02] px-3 py-2 text-[11px] text-slate-400"
        >
          <Info className="h-3.5 w-3.5 shrink-0 text-violet-400" aria-hidden="true" />
          The interviewer sees nothing left to settle. Decompose whenever you want — or keep going.
        </div>
      )}

      <InterviewComposer
        discoveryId={discovery.id}
        agentKind={discovery.agent_kind}
        model={discovery.model ?? ''}
        machineId={discovery.machine_id}
        attachments={discovery.attachments}
        awaiting={awaiting}
        pending={pending}
        disabled={closed}
        value={draft}
        onChange={setDraft}
        onSend={() => send(draft)}
        onAttachmentsChanged={onRefresh}
        inputRef={composerRef}
      />
    </div>
  );
}

export default InterviewColumn;
