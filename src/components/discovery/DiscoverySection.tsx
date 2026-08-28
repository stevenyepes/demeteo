import React, { useEffect, useState } from 'react';
import { Compass } from 'lucide-react';

import { getDiscoveryBoard, summaryOfNew } from '../../lib/discovery';
import { TITLE_MAX_CHARS } from '../../lib/newDiscovery';
import { phaseOfStatus } from '../../lib/discoveryActivity';
import type { DiscoveryBoard, DiscoverySummary } from '../../types';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import EmptyStateCard from '../EmptyStateCard';
import { PipelineListSkeleton } from '../PipelineListSkeleton';
import { DiscoveryCard } from './DiscoveryCard';
import { NewDiscoveryModal } from './NewDiscoveryModal';

interface DiscoverySectionProps {
  projectId: string;
  /** The project's own host: where its repository was cloned, and so what the
   *  modal's machine picker starts on (§4.5). */
  machineId: string;
  discoveries: DiscoverySummary[];
  isLoading: boolean;
  /** A Discovery this session just opened, so the list does not have to wait
   *  for the project fetch to re-run. */
  onCreated: (discovery: DiscoverySummary) => void;
  onOpen: (discoveryId: string, title: string) => void;
}

/**
 * Project Home's Discovery tab (`DISCOVERY_UI_SPEC.md` §1.5): the hero card
 * that opens the modal, then one card per Discovery.
 *
 * The per-Discovery boards are fetched here rather than by Project Home
 * because the ticket set is only ever read by this tab, and this component
 * mounts only when the tab is chosen — so an interview a user never looks at
 * costs no ticket query.
 */
export function DiscoverySection({
  projectId,
  machineId,
  discoveries,
  isLoading,
  onCreated,
  onOpen,
}: DiscoverySectionProps): React.ReactElement {
  const [seed, setSeed] = useState('');
  const [modalOpen, setModalOpen] = useState(false);
  const [boards, setBoards] = useState<Record<string, DiscoveryBoard>>({});
  const [runningTurns, setRunningTurns] = useState<Set<string>>(new Set());
  // One instant for every age on screen, so two cards rendered in the same
  // pass do not read as if they were measured against different clocks.
  const now = Date.now();

  const ids = discoveries.map((d) => d.id).join(',');

  useEffect(() => {
    const list = ids ? ids.split(',') : [];
    if (list.length === 0) {
      setBoards({});
      return;
    }
    let cancelled = false;
    Promise.allSettled(list.map((id) => getDiscoveryBoard(id))).then((results) => {
      if (cancelled) return;
      const next: Record<string, DiscoveryBoard> = {};
      results.forEach((result, i) => {
        if (result.status === 'fulfilled') next[list[i]] = result.value;
        else console.error('DiscoverySection: failed to read a discovery board', result.reason);
      });
      setBoards(next);
    });
    return () => {
      cancelled = true;
    };
  }, [ids]);

  // Liveness, which nothing stores: `DiscoveryStatus` is open/closed, and the
  // pulsing dot on a card claims a turn is running *now*.
  useTauriEvent<{ discovery_id: string; status: string }>(
    'discovery_turn_status',
    ({ discovery_id, status }) => {
      setRunningTurns((prev) => {
        const next = new Set(prev);
        if (phaseOfStatus(status) !== null) next.add(discovery_id);
        else next.delete(discovery_id);
        return next;
      });
    },
  );

  const openModal = () => {
    setModalOpen(true);
  };

  return (
    <div className="flex flex-col gap-5">
      <div className="glass-panel relative overflow-hidden rounded-2xl p-4">
        <div className="flex items-start gap-4">
          <Compass className="mt-1.5 ml-1 h-5 w-5 shrink-0 text-violet-400" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <input
              type="text"
              value={seed}
              onChange={(e) => setSeed(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  openModal();
                }
              }}
              maxLength={TITLE_MAX_CHARS}
              placeholder="Name something you want to think through..."
              aria-label="Name a discovery"
              data-testid="discovery-composer"
              className="w-full border-none bg-transparent p-2 text-sm text-white placeholder-slate-500 focus:outline-none"
            />
            <div className="flex flex-wrap items-center gap-2 px-2 pb-1">
              <span className="font-mono text-[10px] uppercase tracking-wider text-slate-500">
                Interviewer
              </span>
              <span className="text-[11px] text-slate-500">
                chosen per discovery, not from the project default
              </span>
            </div>
          </div>
          <button
            type="button"
            onClick={openModal}
            data-testid="open-new-discovery"
            className="btn-primary mt-1 shrink-0 text-[13px]"
          >
            New discovery
          </button>
        </div>
      </div>

      {isLoading ? (
        <PipelineListSkeleton />
      ) : discoveries.length === 0 ? (
        <EmptyStateCard
          variant="inline"
          icon={Compass}
          title="No discoveries yet"
          description="A discovery is a conversation you can leave and come back to. It reads this repository, runs commands in its own worktree, and ends by proposing tickets you can start one at a time."
        />
      ) : (
        <div className="flex flex-col gap-4">
          {discoveries.map((discovery) => (
            <DiscoveryCard
              key={discovery.id}
              discovery={discovery}
              board={boards[discovery.id] ?? null}
              turnRunning={runningTurns.has(discovery.id)}
              now={now}
              onOpen={onOpen}
            />
          ))}
        </div>
      )}

      {modalOpen && (
        <NewDiscoveryModal
          projectId={projectId}
          machineId={machineId}
          seedTitle={seed}
          onClose={() => setModalOpen(false)}
          onCreated={(discovery) => {
            setModalOpen(false);
            setSeed('');
            onCreated(summaryOfNew(discovery));
          }}
        />
      )}
    </div>
  );
}

export default DiscoverySection;
