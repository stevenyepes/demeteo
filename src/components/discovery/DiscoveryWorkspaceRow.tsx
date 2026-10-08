import React, { useEffect, useLayoutEffect, useState } from 'react';

import { interviewWidthPref } from '../../lib/uiPrefs';

import type { TranscriptBlock } from '../../lib/discoveryInterview';
import type { TicketIndex } from '../../lib/ticketPresentation';
import type {
  Discovery,
  DiscoveryBoard,
  DiscoveryMessageView,
  Machine,
  RunPlacement,
  TicketProgress,
  TicketView,
  WorkflowWithSteps,
} from '../../types';
import { SegmentedControl } from '../ui/SegmentedControl';
import { InterviewCollapsedRail } from './InterviewCollapsedRail';
import { InterviewColumn } from './InterviewColumn';
import { INTERVIEW_WIDTH_VAR, InterviewResizeHandle } from './InterviewResizeHandle';
import { DEFAULT_INTERVIEW_WIDTH, resolveInterviewWidth } from './discoveryLayout';
import { TicketColumn } from './TicketColumn';
import { TicketEditorModal } from './TicketEditorModal';
import { TicketInspector } from './TicketInspector';
import { TicketOverlayPanel } from './TicketOverlayPanel';
import { useDiscoveryColumnLayout } from './useDiscoveryColumnLayout';
import type { DiscoveryStreamStore } from './useDiscoveryStream';

const STACKED_PANE_OPTIONS = [
  { value: 'interview' as const, label: 'Interview' },
  { value: 'tickets' as const, label: 'Tickets' },
];

interface DiscoveryWorkspaceRowProps {
  discovery: Discovery;
  messages: DiscoveryMessageView[];
  blocks: TranscriptBlock[];
  machineLabel: string;
  pending: boolean;
  store: DiscoveryStreamStore;
  onSend: (text: string) => void;
  onRefresh: () => void;

  tickets: TicketView[];
  index: TicketIndex;
  progress: TicketProgress | null;
  selectedId: string | null;
  onSelect: (ticketId: string) => void;

  editing: TicketView | undefined;
  selected: TicketView | undefined;
  workflows: WorkflowWithSteps[];
  workflowName: string | null;
  busy: boolean;
  machines: readonly Machine[];
  /** `null` until the board has answered, which is also when nothing can be
   *  open in the editor. */
  discoveryDefault: RunPlacement | null;
  /** `DiscoveryBoard.local_host`, `null` alongside `discoveryDefault`. */
  localHost: string | null;
  onEditorClose: () => void;
  onInspectorClose: () => void;
  onEditorSaved: (board: DiscoveryBoard) => void;
  onEditorStart: () => void;
  onEditorForceStart: (reason: string) => void;
  onEditorDrop: (reason: string) => void;
  onInspectorStart: () => void;
  onInspectorForceStart: (reason: string) => void;
  onInspectorEdit: () => void;
  onInspectorOpenFeature: (featureId: string) => void;
}

/**
 * The workspace's three panes (`DISCOVERY_UI_SPEC.md` §3), laid out for the
 * width the row actually measures (`implementation-spec.md` §1 AC2–AC4, AC7).
 * The ticket editor is not one of them: it opens as a modal over the window,
 * so the row only ever seats the inspector.
 *
 * `'stacked'`'s pane toggle hides with a class rather than unmounting, so an
 * in-progress interview draft and the graph's zoom state survive a toggle —
 * the same reason `InterviewColumn`/`TicketColumn` grew a `hidden` prop
 * instead of this component conditionally rendering them. The interview's own
 * hide toggle rides the same prop for the same reason.
 *
 * That toggle is a *request*, not the verdict: `'stacked'` shows one pane at a
 * time and already offers the interview as one of them, so honouring a hide
 * there would leave a pane toggle whose Interview position renders nothing.
 * The request is kept rather than cleared, so widening the row restores the
 * collapse the user asked for.
 */
export function DiscoveryWorkspaceRow({
  discovery,
  messages,
  blocks,
  machineLabel,
  pending,
  store,
  onSend,
  onRefresh,
  tickets,
  index,
  progress,
  selectedId,
  onSelect,
  editing,
  selected,
  workflows,
  workflowName,
  busy,
  machines,
  discoveryDefault,
  localHost,
  onEditorClose,
  onInspectorClose,
  onEditorSaved,
  onEditorStart,
  onEditorForceStart,
  onEditorDrop,
  onInspectorStart,
  onInspectorForceStart,
  onInspectorEdit,
  onInspectorOpenFeature,
}: DiscoveryWorkspaceRowProps): React.ReactElement {
  const [interviewHidden, setInterviewHidden] = useState(false);
  const [interviewWidth, setInterviewWidth] = useState(DEFAULT_INTERVIEW_WIDTH);
  const { rowEl, setRowEl, rowSize, layoutMode } = useDiscoveryColumnLayout(
    interviewHidden,
    interviewWidth,
  );
  const [stackedPane, setStackedPane] = useState<'interview' | 'tickets'>('interview');

  useEffect(() => {
    let cancelled = false;
    void interviewWidthPref.read().then((width) => {
      if (!cancelled) setInterviewWidth(width);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const renderedInterviewWidth = resolveInterviewWidth(interviewWidth, rowSize?.width ?? 0);

  useLayoutEffect(() => {
    rowEl?.style.setProperty(INTERVIEW_WIDTH_VAR, `${renderedInterviewWidth}px`);
  }, [rowEl, renderedInterviewWidth]);

  const commitInterviewWidth = (width: number) => {
    setInterviewWidth(width);
    interviewWidthPref.write(width);
  };

  const overlaid = layoutMode !== 'three-up';
  const stacked = layoutMode === 'stacked';
  const interviewCollapsed = interviewHidden && !stacked;

  const editor = editing && discoveryDefault && localHost !== null && (
    <TicketEditorModal
      key={editing.ticket.id}
      view={editing}
      index={index}
      siblings={tickets}
      workflows={workflows}
      machines={machines}
      discoveryDefault={discoveryDefault}
      localHost={localHost}
      busy={busy}
      onClose={onEditorClose}
      onSaved={onEditorSaved}
      onRefresh={onRefresh}
      onStart={onEditorStart}
      onForceStart={onEditorForceStart}
      onDrop={onEditorDrop}
    />
  );

  const inspector = selected && (
    <TicketInspector
      key={selected.ticket.id}
      view={selected}
      index={index}
      workflowName={workflowName}
      machines={machines}
      busy={busy}
      onStart={onInspectorStart}
      onForceStart={onInspectorForceStart}
      onEdit={onInspectorEdit}
      onOpenFeature={onInspectorOpenFeature}
      onClose={onInspectorClose}
    />
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {stacked && (
        <div className="flex shrink-0 justify-center border-b border-white/5 bg-[#0b0d12]/40 px-3 py-2">
          <SegmentedControl
            options={STACKED_PANE_OPTIONS}
            value={stackedPane}
            onChange={setStackedPane}
            size="sm"
            ariaLabel="Workspace pane"
          />
        </div>
      )}

      <div
        className="relative flex min-h-0 flex-1"
        ref={setRowEl}
        data-testid="discovery-workspace-row"
      >
        <InterviewColumn
          discovery={discovery}
          messages={messages}
          blocks={blocks}
          machineLabel={machineLabel}
          pending={pending}
          store={store}
          onSend={onSend}
          onRefresh={onRefresh}
          widthMode={stacked ? 'full' : 'fixed'}
          hidden={stacked ? stackedPane !== 'interview' : interviewCollapsed}
          onHide={stacked ? undefined : () => setInterviewHidden(true)}
        />

        {!stacked && !interviewCollapsed && (
          <InterviewResizeHandle
            rowEl={rowEl}
            width={renderedInterviewWidth}
            onCommit={commitInterviewWidth}
          />
        )}

        {interviewCollapsed && (
          <InterviewCollapsedRail onShow={() => setInterviewHidden(false)} pending={pending} />
        )}

        <TicketColumn
          tickets={tickets}
          index={index}
          progress={progress}
          selectedId={selectedId}
          onSelect={onSelect}
          hidden={stacked && stackedPane !== 'tickets'}
        />

        {!overlaid && inspector}

        {overlaid && inspector && (
          <TicketOverlayPanel onClose={onInspectorClose} label="Ticket inspector">
            {inspector}
          </TicketOverlayPanel>
        )}
      </div>

      {editor}
    </div>
  );
}

export default DiscoveryWorkspaceRow;
