import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { MessageSquare } from 'lucide-react';

import { listAskThreads } from '../../lib/ask';
import { formatError } from '../../lib/errors';
import { DEFAULT_SESSION_FILTER, filterSessions } from '../../lib/sessionFilter';
import { useLiveAskTurns } from '../../hooks/useLiveAskTurns';
import type { AskThread } from '../../types';
import EmptyStateCard from '../EmptyStateCard';
import { PipelineListSkeleton } from '../PipelineListSkeleton';
import { SessionFilterBar } from '../SessionFilterBar';
import { AskThreadCard } from './AskThreadCard';
import { NewAskThreadModal } from './NewAskThreadModal';

interface AskSectionProps {
  projectId: string;
  /** The project's own host: where its repository was cloned, and so what the
   *  modal's machine picker starts on — `DiscoverySection`'s convention. */
  machineId: string;
  /** Names the modal's eyebrow label; falls back to a generic string. */
  projectName?: string;
  onOpen: (threadId: string) => void;
}

/**
 * Project Home's Ask tab: the hero composer, then one card per thread.
 *
 * The tab used to be a launcher — choosing it navigated straight into
 * whichever thread happened to be open, and the only way to reach any other
 * was a "Threads ▾" dropdown inside the workspace. That dropdown was a
 * switcher, not a list: no filter, no spend, and behind the very view you
 * were trying to leave, so a project with a dozen threads had no surface that
 * showed them. This list is that surface, and the dropdown is gone rather
 * than kept beside it — two ways to reach a thread, one of them worse, is the
 * shape that put the tab in the state it was in.
 *
 * The thread list is fetched here rather than by Project Home — this component
 * mounts only when the tab is chosen, so a project whose Ask tab is never
 * opened costs no query. Discovery's list is loaded the other way round
 * because the pipelines tab reads it too.
 */
export function AskSection({
  projectId,
  machineId,
  projectName,
  onOpen,
}: AskSectionProps): React.ReactElement {
  const [seed, setSeed] = useState('');
  const [modalOpen, setModalOpen] = useState(false);
  const [threads, setThreads] = useState<AskThread[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState(DEFAULT_SESSION_FILTER);
  const runningTurns = useLiveAskTurns();
  // One instant for every age on screen, so two cards rendered in the same
  // pass do not read as if they were measured against different clocks.
  const now = Date.now();

  const load = useCallback(async () => {
    try {
      setThreads(await listAskThreads(projectId));
      setError(null);
    } catch (cause) {
      setError(formatError(cause));
    }
  }, [projectId]);

  useEffect(() => {
    setThreads(null);
    void load();
  }, [load]);

  const visible = useMemo(
    () => filterSessions(threads ?? [], filter),
    [threads, filter],
  );

  return (
    <div className="flex flex-col gap-5">
      <div className="glass-panel relative overflow-hidden rounded-2xl p-4">
        <div className="flex items-start gap-4">
          <MessageSquare className="mt-1.5 ml-1 h-5 w-5 shrink-0 text-violet-400" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <input
              type="text"
              value={seed}
              onChange={(e) => setSeed(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  setModalOpen(true);
                }
              }}
              placeholder="Ask something about this codebase..."
              aria-label="Name an Ask thread"
              data-testid="ask-composer"
              className="w-full border-none bg-transparent p-2 text-sm text-white placeholder-slate-500 focus:outline-none"
            />
            <div className="flex flex-wrap items-center gap-2 px-2 pb-1">
              <span className="font-mono text-[10px] uppercase tracking-wider text-slate-500">
                Agent
              </span>
              <span className="text-[11px] text-slate-500">
                chosen per thread, not from the project default
              </span>
            </div>
          </div>
          <button
            type="button"
            onClick={() => setModalOpen(true)}
            data-testid="open-new-ask-thread"
            className="btn-primary mt-1 shrink-0 text-[13px]"
          >
            New thread
          </button>
        </div>
      </div>

      {error && (
        <p role="alert" className="font-mono text-[11px] text-ruby-200">
          {error}
        </p>
      )}

      {threads === null ? (
        <PipelineListSkeleton />
      ) : threads.length === 0 ? (
        <EmptyStateCard
          variant="inline"
          icon={MessageSquare}
          title="No threads yet"
          description="Ask reads this repository in its own read-only worktree and answers in prose, with a canvas of the parts it touched. Nothing it does can change the code."
        />
      ) : (
        <>
          <SessionFilterBar
            value={filter}
            onChange={setFilter}
            sessions={threads}
            resultCount={visible.length}
            noun="threads"
            testId="ask-filter-bar"
          />
          <div className="flex flex-col gap-4">
            {visible.map((thread) => (
              <AskThreadCard
                key={thread.id}
                thread={thread}
                turnRunning={runningTurns.has(thread.id)}
                now={now}
                onOpen={onOpen}
              />
            ))}
          </div>
        </>
      )}

      {modalOpen && (
        <NewAskThreadModal
          projectId={projectId}
          machineId={machineId}
          projectName={projectName}
          seedTitle={seed}
          onClose={() => setModalOpen(false)}
          onCreated={(thread) => {
            setModalOpen(false);
            setSeed('');
            onOpen(thread.id);
          }}
        />
      )}
    </div>
  );
}

export default AskSection;
