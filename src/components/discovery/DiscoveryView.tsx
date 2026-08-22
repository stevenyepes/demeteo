import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  EVENT_DISCOVERY_TURN_COMPLETED,
  EVENT_DISCOVERY_TURN_STATUS,
  closeDiscovery,
  decomposeDiscovery,
  discardProposal,
  dropTicket,
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
import { formatError } from '../../lib/errors';
import { listMachines } from '../../lib/machines';
import { indexTickets } from '../../lib/ticketPresentation';
import { formatDuration } from '../../lib/utils';
import { listWorkflows } from '../../lib/workflows';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import type {
  DecomposeProposal,
  DiscoveryBoard,
  DiscoveryDetail,
  DiscoveryMessageView,
  WorkflowWithSteps,
} from '../../types';
import { DecomposeModal } from './DecomposeModal';
import { DiscoveryWorkspaceHeader } from './DiscoveryWorkspaceHeader';
import { InterviewColumn } from './InterviewColumn';
import { PendingProposalNotice } from './PendingProposalNotice';
import { TicketColumn } from './TicketColumn';
import { TicketEditorDrawer } from './TicketEditorDrawer';
import { TicketInspector } from './TicketInspector';
import { TurnCompleteToast } from './TurnCompleteToast';
import { useDiscoveryStream } from './useDiscoveryStream';

interface DiscoveryViewProps {
  discoveryId: string;
  /** Carried on the view so the header can name the Discovery before
   *  `discovery_get` answers. */
  discoveryTitle: string;
  /** Opens a started ticket's Feature. */
  onOpenFeature?: (featureId: string, featureTitle: string) => void;
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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [toast, setToast] = useState<{ title: string; detail: string } | null>(null);
  // Which settled turns had to carry the transcript themselves. Only the
  // completion event knows, and nothing stores it, so this is what the
  // workspace heard while it was open and never a claim about older turns.
  const [reseeded, setReseeded] = useState<ReadonlySet<string>>(() => new Set());
  const [machineName, setMachineName] = useState<string | null>(null);
  const [workflows, setWorkflows] = useState<WorkflowWithSteps[]>([]);
  const [proposal, setProposal] = useState<DecomposeProposal | null>(null);
  const [decomposing, setDecomposing] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);

  // A turn this view found already running rather than one it started. Both
  // refs answer questions no event can: the first, whether this discovery's
  // backend answer has been read yet; the second, whether the end of that turn
  // is this view's to go and fetch, since the completion it would otherwise
  // learn from was reported to a component that is gone.
  const adopted = useRef<string | null>(null);
  const awaitingAdopted = useRef(false);

  const { store, begin, end } = useDiscoveryStream();

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
      .then((list) => {
        if (!cancelled) setWorkflows(list ?? []);
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

  /**
   * Whether a turn or a pass was already under way when this view opened
   * (`turn_running`). Read once per discovery, and only at the open: after
   * that the events are the authority, and a second read racing a turn that
   * has just ended would leave the composer waiting on nothing.
   *
   * A pass adopted this way reports its end on the status event alone — the
   * completion event belongs to turns, and the promise `decompose` returns was
   * handed to a component that has since unmounted — so this view has to go
   * and fetch what the pass produced itself.
   */
  useEffect(() => {
    if (!detail || adopted.current === discoveryId) return;
    adopted.current = discoveryId;
    if (!detail.turn_running) return;
    awaitingAdopted.current = true;
    setPending(true);
    begin(discoveryId);
  }, [detail, discoveryId, begin]);

  useTauriEvent<DiscoveryTurnStatusPayload>(
    EVENT_DISCOVERY_TURN_STATUS,
    ({ discovery_id, status, reason }) => {
      if (discovery_id !== discoveryId) return;
      setPending(status === 'running');
      if (status === 'running') begin(discoveryId);
      if (status === 'error' && reason) setActionError(reason);
      if (status !== 'running' && awaitingAdopted.current) {
        awaitingAdopted.current = false;
        end(discoveryId);
        void refresh();
      }
    },
    [discoveryId, begin, end, refresh],
  );

  useTauriEvent<DiscoveryTurnCompletedPayload>(
    EVENT_DISCOVERY_TURN_COMPLETED,
    (payload) => {
      if (payload.discovery_id !== discoveryId) return;
      setPending(false);
      end(discoveryId);
      if (payload.reseeded && payload.message_id !== null) {
        const id = payload.message_id;
        setReseeded((current) => new Set(current).add(id));
      }
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
    [discoveryId, refresh, end],
  );

  const tickets = useMemo(() => board?.tickets ?? [], [board]);
  const index = useMemo(() => indexTickets(tickets), [tickets]);
  const blocks = useMemo(
    () => buildTranscript(detail?.messages ?? [], reseeded),
    [detail?.messages, reseeded],
  );

  useEffect(() => {
    if (selectedId !== null && index.has(selectedId)) return;
    setSelectedId(tickets.length > 0 ? tickets[0].ticket.id : null);
  }, [tickets, index, selectedId]);

  async function send(text: string) {
    setActionError(null);
    // From the click, not from the `running` status a round trip later: the
    // bubble mounts on `sending` and would otherwise count from zero once the
    // backend answered.
    begin(discoveryId);
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

  /**
   * Ask for a proposal (§5.1). The **user** decides when, and it is available
   * from the first turn: `nothing_left_to_settle` is advisory and is never a
   * gate, because a model that keeps finding one more question would otherwise
   * hold the interview open.
   *
   * The pass streams through the interview's own events, so the transcript
   * shows the agent working while this awaits.
   */
  async function decompose() {
    setDecomposing(true);
    setActionError(null);
    try {
      setProposal(await decomposeDiscovery(discoveryId));
    } catch (cause) {
      setActionError(formatError(cause));
    } finally {
      setDecomposing(false);
      await refresh();
    }
  }

  const pendingProposal = detail?.pending_proposal ?? null;

  const selected = selectedId !== null ? index.get(selectedId) : undefined;
  const editing = editingId !== null ? index.get(editingId) : undefined;
  const workflowNames = useMemo(
    () => Object.fromEntries(workflows.map((workflow) => [workflow.id, workflow.name])),
    [workflows],
  );

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
        turnCount={detail.messages.length}
        turnRunning={pending || sending || decomposing}
        busy={busy || decomposing}
        onToggleOpen={() =>
          void runAction(() =>
            detail.discovery.status === 'open'
              ? closeDiscovery(discoveryId)
              : reopenDiscovery(discoveryId),
          )
        }
        onDecompose={() => void decompose()}
        decomposing={decomposing}
      />

      {actionError && (
        <p
          role="alert"
          className="m-0 shrink-0 border-b border-ruby-500/20 bg-ruby-500/5 px-6 py-2 font-mono text-[11px] text-ruby-200"
        >
          {actionError}
        </p>
      )}

      {pendingProposal && proposal === null && (
        <PendingProposalNotice
          proposal={pendingProposal}
          busy={busy || decomposing}
          onReview={() => setProposal(pendingProposal)}
          onDiscard={() => void runAction(() => discardProposal(discoveryId))}
        />
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

        <TicketColumn
          tickets={tickets}
          index={index}
          progress={board?.progress ?? null}
          selectedId={selectedId}
          onSelect={setSelectedId}
        />

        {editing ? (
          <TicketEditorDrawer
            key={editing.ticket.id}
            view={editing}
            index={index}
            siblings={tickets}
            workflows={workflows}
            machineId={detail.discovery.machine_id}
            busy={busy}
            onClose={() => setEditingId(null)}
            onSaved={setBoard}
            onRefresh={() => void refresh()}
            onStart={() => void runAction(() => startTicket(editing.ticket.id))}
            onForceStart={(reason) =>
              void runAction(() => forceStartTicket(editing.ticket.id, reason))
            }
            onDrop={(reason) => void runAction(() => dropTicket(editing.ticket.id, reason))}
          />
        ) : (
          selected && (
            <TicketInspector
              key={selected.ticket.id}
              view={selected}
              index={index}
              workflowName={
                selected.ticket.workflow_id
                  ? (workflowNames[selected.ticket.workflow_id] ?? null)
                  : null
              }
              busy={busy}
              onStart={() => void runAction(() => startTicket(selected.ticket.id))}
              onForceStart={(reason) =>
                void runAction(() => forceStartTicket(selected.ticket.id, reason))
              }
              onEdit={() => setEditingId(selected.ticket.id)}
              onOpenFeature={(featureId) => onOpenFeature?.(featureId, selected.ticket.title)}
            />
          )
        )}
      </div>

      {proposal && (
        <DecomposeModal
          proposal={proposal}
          index={index}
          onClose={() => setProposal(null)}
          onApplied={(applied) => {
            setBoard(applied);
            setProposal(null);
            void refresh();
          }}
        />
      )}

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
