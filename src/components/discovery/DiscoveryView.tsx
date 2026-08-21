import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Compass, Kanban, Workflow } from 'lucide-react';

import {
  EVENT_DISCOVERY_TURN_COMPLETED,
  EVENT_DISCOVERY_TURN_STATUS,
  closeDiscovery,
  forceStartTicket,
  getDiscovery,
  getDiscoveryBoard,
  reopenDiscovery,
  sendDiscoveryTurn,
  startTicket,
  type DiscoveryTurnCompletedPayload,
  type DiscoveryTurnStatusPayload,
} from '../../lib/discovery';
import { buildTranscript } from '../../lib/discoveryInterview';
import { progressSegments, progressText } from '../../lib/discoveryProgress';
import { formatError } from '../../lib/errors';
import { listMachines } from '../../lib/machines';
import { indexTickets } from '../../lib/ticketPresentation';
import { formatDuration } from '../../lib/utils';
import { listWorkflows } from '../../lib/workflows';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import type { DiscoveryBoard, DiscoveryDetail, DiscoveryMessageView } from '../../types';
import EmptyStateCard from '../EmptyStateCard';
import { SegmentedControl } from '../ui/SegmentedControl';
import { ColumnSubHeader } from './ColumnSubHeader';
import { DiscoveryWorkspaceHeader } from './DiscoveryWorkspaceHeader';
import { InterviewColumn } from './InterviewColumn';
import { TicketBoard } from './TicketBoard';
import { TicketGraph } from './TicketGraph';
import { TicketInspector } from './TicketInspector';
import { TicketProgressBar } from './TicketProgressBar';
import { TurnCompleteToast } from './TurnCompleteToast';
import { useDiscoveryStream } from './useDiscoveryStream';

type TicketViewMode = 'graph' | 'board';

const VIEW_OPTIONS = [
  { value: 'graph' as const, label: 'Graph', icon: Workflow },
  { value: 'board' as const, label: 'Board', icon: Kanban },
];

interface DiscoveryViewProps {
  discoveryId: string;
  /** Carried on the view so the header can name the Discovery before
   *  `discovery_get` answers. */
  discoveryTitle: string;
  /** Opens a started ticket's Feature. */
  onOpenFeature?: (featureId: string, featureTitle: string) => void;
  /** Phase 7's proposed-changes review, when it exists. */
  onDecompose?: () => void;
}

/**
 * One Discovery's workspace: the interview, the tickets it proposed, and the
 * standing of whichever one is selected (`DISCOVERY_UI_SPEC.md` §3).
 *
 * **The graph and the board read one `discovery_board` call.** §9.2 makes that
 * the whole reason the second view is safe to have — a ticket cannot be done
 * on the board and blocked in the graph when there is only one answer between
 * them, and nothing here recomputes a lane the backend already derived.
 */
export function DiscoveryView({
  discoveryId,
  discoveryTitle,
  onOpenFeature,
  onDecompose,
}: DiscoveryViewProps): React.ReactElement {
  const [detail, setDetail] = useState<DiscoveryDetail | null>(null);
  const [board, setBoard] = useState<DiscoveryBoard | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState(false);
  // The turn is in flight from the click, not from the first status event —
  // otherwise a second click lands before the backend has said anything.
  const [sending, setSending] = useState(false);
  const [mode, setMode] = useState<TicketViewMode>('graph');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [toast, setToast] = useState<{ title: string; detail: string } | null>(null);
  const [machineName, setMachineName] = useState<string | null>(null);
  const [workflowNames, setWorkflowNames] = useState<Record<string, string>>({});

  const { store, reset } = useDiscoveryStream();

  const refresh = useCallback(async () => {
    const [detailResult, boardResult] = await Promise.allSettled([
      getDiscovery(discoveryId),
      getDiscoveryBoard(discoveryId),
    ]);
    if (detailResult.status === 'fulfilled') {
      setDetail(detailResult.value);
      setError(null);
    } else {
      setError(formatError(detailResult.reason));
    }
    if (boardResult.status === 'fulfilled') setBoard(boardResult.value);
  }, [discoveryId]);

  useEffect(() => {
    setDetail(null);
    setBoard(null);
    setSelectedId(null);
    setError(null);
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let cancelled = false;
    listWorkflows()
      .then((workflows) => {
        if (cancelled) return;
        setWorkflowNames(Object.fromEntries(workflows.map((w) => [w.id, w.name])));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const machineId = detail?.discovery.machine_id;
  useEffect(() => {
    if (!machineId) return;
    let cancelled = false;
    listMachines()
      .then((machines) => {
        if (cancelled) return;
        setMachineName(machines.find((m) => m.id === machineId)?.name ?? null);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [machineId]);

  useTauriEvent<DiscoveryTurnStatusPayload>(
    EVENT_DISCOVERY_TURN_STATUS,
    ({ discovery_id, status, reason }) => {
      if (discovery_id !== discoveryId) return;
      setPending(status === 'running');
      if (status === 'running') reset(discoveryId);
      if (status === 'error' && reason) setActionError(reason);
    },
    [discoveryId, reset],
  );

  useTauriEvent<DiscoveryTurnCompletedPayload>(
    EVENT_DISCOVERY_TURN_COMPLETED,
    (payload) => {
      if (payload.discovery_id !== discoveryId) return;
      setPending(false);
      reset(discoveryId);
      void refresh();
      if (payload.ending === 'success') {
        setToast({
          title: 'Turn complete',
          detail: `${payload.title} · ${formatDuration(payload.duration_ms / 1000)}`,
        });
      } else if (payload.reason) {
        setActionError(payload.reason);
      }
    },
    [discoveryId, refresh, reset],
  );

  const tickets = useMemo(() => board?.tickets ?? [], [board]);
  const index = useMemo(() => indexTickets(tickets), [tickets]);
  const blocks = useMemo(
    () => buildTranscript(detail?.messages ?? []),
    [detail?.messages],
  );

  useEffect(() => {
    if (selectedId !== null && index.has(selectedId)) return;
    setSelectedId(tickets.length > 0 ? tickets[0].ticket.id : null);
  }, [tickets, index, selectedId]);

  async function send(text: string) {
    setActionError(null);
    setSending(true);
    try {
      const stored = await sendDiscoveryTurn(discoveryId, text);
      const appended: DiscoveryMessageView = {
        ...stored,
        prose: stored.content,
        question: null,
        nothing_left_to_settle: false,
        question_error: null,
      };
      setDetail((current) =>
        current ? { ...current, messages: [...current.messages, appended] } : current,
      );
      setPending(true);
    } catch (cause) {
      setActionError(formatError(cause));
    } finally {
      setSending(false);
    }
  }

  async function runAction(action: () => Promise<unknown>) {
    setBusy(true);
    setActionError(null);
    try {
      await action();
      await refresh();
    } catch (cause) {
      setActionError(formatError(cause));
    } finally {
      setBusy(false);
    }
  }

  const selected = selectedId !== null ? index.get(selectedId) : undefined;
  const progress = board ? progressText(board.progress) : null;
  const segments = board ? progressSegments(board.progress) : null;

  if (!detail) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center bg-[#0a0c10]">
        {error ? (
          <p role="alert" className="font-mono text-xs text-ruby-200">
            {error}
          </p>
        ) : (
          <p className="font-mono text-xs text-slate-500">Opening {discoveryTitle}…</p>
        )}
      </div>
    );
  }

  return (
    <div className="relative flex flex-1 flex-col overflow-hidden bg-[#0a0c10]">
      <DiscoveryWorkspaceHeader
        discovery={detail.discovery}
        board={board}
        turnCount={blocks.length}
        turnRunning={pending || sending}
        busy={busy}
        onToggleOpen={() =>
          void runAction(() =>
            detail.discovery.status === 'open'
              ? closeDiscovery(discoveryId)
              : reopenDiscovery(discoveryId),
          )
        }
        onDecompose={onDecompose}
      />

      {actionError && (
        <p
          role="alert"
          className="m-0 shrink-0 border-b border-ruby-500/20 bg-ruby-500/5 px-6 py-2 font-mono text-[11px] text-ruby-200"
        >
          {actionError}
        </p>
      )}

      <div className="flex min-h-0 flex-1">
        <InterviewColumn
          discovery={detail.discovery}
          messages={detail.messages}
          blocks={blocks}
          machineLabel={machineName ?? detail.discovery.machine_id}
          pending={pending || sending}
          store={store}
          onSend={(text) => void send(text)}
          onRefresh={() => void refresh()}
        />

        <div className="flex min-w-0 min-h-0 flex-1 flex-col">
          <ColumnSubHeader
            left={
              <SegmentedControl
                options={VIEW_OPTIONS}
                value={mode}
                onChange={setMode}
                size="sm"
                ariaLabel="Ticket view"
              />
            }
          >
            {progress && segments && (
              <>
                <span className="font-mono text-[10px] text-slate-400">{progress}</span>
                <TicketProgressBar
                  landedPct={segments.landedPct}
                  inFlightPct={segments.inFlightPct}
                  title={progress}
                  className="w-24 shrink-0"
                />
              </>
            )}
          </ColumnSubHeader>

          <div className="relative min-h-0 flex-1">
            {tickets.length === 0 ? (
              <div className="absolute inset-0 overflow-y-auto bg-[#050608] p-6">
                <EmptyStateCard
                  variant="inline"
                  icon={Compass}
                  title="No tickets yet"
                  description="This discovery has proposed nothing so far. Decompose the interview when the shape is settled, and the tickets it produces appear here with their edges."
                />
              </div>
            ) : mode === 'graph' ? (
              <TicketGraph
                tickets={tickets}
                index={index}
                selectedId={selectedId}
                onSelect={setSelectedId}
              />
            ) : (
              <TicketBoard
                tickets={tickets}
                index={index}
                selectedId={selectedId}
                onSelect={setSelectedId}
              />
            )}
          </div>
        </div>

        {selected && (
          <TicketInspector
            key={selected.ticket.id}
            view={selected}
            index={index}
            workflowName={
              selected.ticket.workflow_id ? (workflowNames[selected.ticket.workflow_id] ?? null) : null
            }
            busy={busy}
            onStart={() => void runAction(() => startTicket(selected.ticket.id))}
            onForceStart={(reason) =>
              void runAction(() => forceStartTicket(selected.ticket.id, reason))
            }
            onOpenFeature={(featureId) => onOpenFeature?.(featureId, selected.ticket.title)}
          />
        )}
      </div>

      {toast && (
        <TurnCompleteToast
          title={toast.title}
          detail={toast.detail}
          onDismiss={() => setToast(null)}
        />
      )}
    </div>
  );
}

export default DiscoveryView;
