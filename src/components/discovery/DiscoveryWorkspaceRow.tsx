import React, { useState } from 'react';

import type { TranscriptBlock } from '../../lib/discoveryInterview';
import type { TicketIndex } from '../../lib/ticketPresentation';
import type {
  Discovery,
  DiscoveryBoard,
  DiscoveryMessageView,
  TicketProgress,
  TicketView,
  WorkflowWithSteps,
} from '../../types';
import { SegmentedControl } from '../ui/SegmentedControl';
import { InterviewCollapsedRail } from './InterviewCollapsedRail';
import { InterviewColumn } from './InterviewColumn';
import { TicketColumn } from './TicketColumn';
import { TicketEditorDrawer } from './TicketEditorDrawer';
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
  machineId: string;
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
  machineId,
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
  const { setRowEl, layoutMode } = useDiscoveryColumnLayout(interviewHidden);
  const [stackedPane, setStackedPane] = useState<'interview' | 'tickets'>('interview');

  const overlaid = layoutMode !== 'three-up';
  const stacked = layoutMode === 'stacked';
  const interviewCollapsed = interviewHidden && !stacked;

  const pane = editing ? (
    <TicketEditorDrawer
      key={editing.ticket.id}
      view={editing}
      index={index}
      siblings={tickets}
      workflows={workflows}
      machineId={machineId}
      busy={busy}
      onClose={onEditorClose}
      onSaved={onEditorSaved}
      onRefresh={onRefresh}
      onStart={onEditorStart}
      onForceStart={onEditorForceStart}
      onDrop={onEditorDrop}
    />
  ) : (
    selected && (
      <TicketInspector
        key={selected.ticket.id}
        view={selected}
        index={index}
        workflowName={workflowName}
        busy={busy}
        onStart={onInspectorStart}
        onForceStart={onInspectorForceStart}
        onEdit={onInspectorEdit}
        onOpenFeature={onInspectorOpenFeature}
        onClose={onInspectorClose}
      />
    )
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

      <div className="flex min-h-0 flex-1" ref={setRowEl} data-testid="discovery-workspace-row">
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

        {!overlaid && pane}
      </div>

      {overlaid && pane && (
        <TicketOverlayPanel
          widthPx={editing ? 760 : 360}
          onClose={editing ? onEditorClose : onInspectorClose}
          label={editing ? 'Ticket editor' : 'Ticket inspector'}
        >
          {pane}
        </TicketOverlayPanel>
      )}
    </div>
  );
}

export default DiscoveryWorkspaceRow;
