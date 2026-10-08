import type { ReactNode } from 'react';
import type { HarnessBaseline, RemoteRunMirror, RunEvent } from '../../types';
import type { HarnessEvidence } from '../../lib/harnessVerdict';
import { TERMINAL_STATUSES } from '../../lib/runStatus';
import { relativeTime } from '../../lib/utils';
import { BootstrapStepper, type BootstrapPhaseView } from '../BootstrapStepper';
import { HarnessGateTable } from '../HarnessGateTable';
import { RemoteGateActions, ReinjectCredentials } from '../RemoteRunActions';
import { bucketFor } from '../../lib/remoteRunBuckets';
import type { RunLayoutMode } from '../runLayout';
import { ActivityPanel } from './ActivityPanel';
import { InitialPromptPanel } from './InitialPromptPanel';

interface RunMetaColumnProps {
  runLayout: RunLayoutMode;
  /** The track's width in px when it is one (`metaTrackWidth`), `null` stacked. */
  widthPx: number | null;
  setMetaChromeEl: (el: HTMLDivElement | null) => void;
  remoteRun: RemoteRunMirror | null;
  remoteMachineName: string | null;
  /** The unified run-event feed, whichever transport filled it. */
  runEvents: RunEvent[];
  activityOpen: boolean;
  onActivityOpenChange: (open: boolean) => void;
  onRunEvents: (events: RunEvent[]) => void;
  onRemoteResolved: () => void;
  /** Display status of the local run — decides whether its feed can still grow. */
  runStatus: string;
  showBootstrap: boolean;
  bootstrapPhases: BootstrapPhaseView[];
  harnessBaseline: HarnessBaseline | null;
  harnessEvidence: HarnessEvidence | null;
  harnessOpen: boolean;
  onHarnessOpenChange: (open: boolean) => void;
  /** Stacked only: the launch prompt joins the chip row. Split, it keeps its
   *  full-width bar above the run, which `FeatureDetail` renders. */
  featureDescription: string;
  /** Stacked only: controls that end the chip row — the Graph|Timeline toggle,
   *  which otherwise spent a row of its own under these chips. */
  trailing?: ReactNode;
}

/**
 * At 'split' the meta panels take their own track, so a wide window gains a
 * second column instead of a longer scroll — chronology still reads
 * top-to-bottom and prose keeps its measure. At 'stacked' they collapse to one
 * row of chips above the graph, which is the only case where they count as its
 * chrome — hence the ref is registered on that branch alone.
 */
export function RunMetaColumn({
  runLayout,
  widthPx,
  setMetaChromeEl,
  remoteRun,
  remoteMachineName,
  runEvents,
  activityOpen,
  onActivityOpenChange,
  onRunEvents,
  onRemoteResolved,
  runStatus,
  showBootstrap,
  bootstrapPhases,
  harnessBaseline,
  harnessEvidence,
  harnessOpen,
  onHarnessOpenChange,
  featureDescription,
  trailing,
}: RunMetaColumnProps) {
  const remoteTerminal = remoteRun !== null && TERMINAL_STATUSES.includes(remoteRun.status);
  // A local run with nothing in its feed yet gets no panel at all: the push
  // starts on mount and is never backfilled, so an empty Activity block on a
  // finished feature would be a permanent, unexplained blank.
  const showActivity = remoteRun !== null || runEvents.length > 0;
  const remoteBucket = remoteRun ? bucketFor(remoteRun.status) : null;
  const remoteActions =
    remoteRun && remoteBucket === 'parked' ? (
      <RemoteGateActions run={remoteRun} onResolved={onRemoteResolved} />
    ) : remoteRun && remoteBucket === 'needs_credentials' ? (
      <ReinjectCredentials run={remoteRun} onResolved={onRemoteResolved} />
    ) : null;

  if (runLayout === 'stacked') {
    /* One wrapping row instead of three full-width bars: closed, each of them
       was a 54px card carrying a dozen characters of summary, and together with
       the sync caption and the toggle row they put the graph a third of the
       window down. An open chip's body is `order-last basis-full` (see the
       `chip` variant of `Disclosure`), so it lands under the whole row. The
       `pb-4` is padding, not margin, so `useRunColumnLayout` measures it. */
    return (
      <div
        data-testid="run-meta-column"
        ref={setMetaChromeEl}
        className="flex w-full min-w-0 flex-wrap items-center gap-2 pb-4"
      >
        <InitialPromptPanel featureDescription={featureDescription} variant="chip" />
        {showActivity && (
          <ActivityPanel
            variant="chip"
            events={runEvents}
            remote={
              remoteRun
                ? {
                    run: remoteRun,
                    machineName: remoteMachineName ?? remoteRun.machine_id,
                    onEvents: onRunEvents,
                  }
                : null
            }
            terminal={remoteRun ? remoteTerminal : TERMINAL_STATUSES.includes(runStatus)}
            open={activityOpen}
            onOpenChange={onActivityOpenChange}
          />
        )}
        <HarnessGateTable
          variant="chip"
          baseline={harnessBaseline}
          evidence={harnessEvidence}
          open={harnessOpen}
          onOpenChange={onHarnessOpenChange}
        />
        {remoteRun && (
          <span className="px-1 font-mono text-[10px] text-slate-500">
            {remoteTerminal
              ? `Final state synced ${relativeTime(remoteRun.updated_at)}`
              : `Status last synced ${relativeTime(remoteRun.updated_at)}`}
          </span>
        )}
        {trailing && <div className="ml-auto flex items-center gap-3">{trailing}</div>}
        {remoteActions && <div className="order-last flex basis-full justify-end">{remoteActions}</div>}
        {showBootstrap && (
          <div className="order-last basis-full">
            <BootstrapStepper phases={bootstrapPhases} />
          </div>
        )}
      </div>
    );
  }

  return (
    <div
      data-testid="run-meta-column"
      // Split, this is one of three full-height tracks and scrolls itself. The
      // width arrives as a number rather than a class because `runLayout.ts`
      // has to subtract it to size the pane pair beside it, and a share spelled
      // once in CSS and once in TypeScript is two answers waiting to disagree.
      style={widthPx === null ? undefined : { width: widthPx }}
      className="flex h-full min-h-0 shrink-0 flex-col overflow-y-auto overflow-x-hidden"
    >
      {showActivity && (
        <div className="mb-6 w-full shrink-0 space-y-1.5">
          <ActivityPanel
            events={runEvents}
            remote={
              remoteRun
                ? {
                    run: remoteRun,
                    machineName: remoteMachineName ?? remoteRun.machine_id,
                    onEvents: onRunEvents,
                  }
                : null
            }
            terminal={remoteRun ? remoteTerminal : TERMINAL_STATUSES.includes(runStatus)}
            open={activityOpen}
            onOpenChange={onActivityOpenChange}
          />
          {remoteRun && (
            <div className="flex items-center justify-between gap-3 px-1">
              {/* The mirror's own freshness, which is a different poll from the
                  event tail the panel names: this one backs off to 48s and stops
                  while the window is hidden, so it states when it last landed
                  rather than an interval it would spend most of its life not
                  keeping. */}
              <p className="text-[10px] font-mono text-slate-500">
                {remoteTerminal
                  ? `Final state synced ${relativeTime(remoteRun.updated_at)}`
                  : `Status last synced ${relativeTime(remoteRun.updated_at)}`}
              </p>
              {/* Same grouping as the Runs inbox: `over-budget` parks
                  too, and RemoteGateActions already renders its
                  no-gate explanation for it. */}
              {remoteActions}
            </div>
          )}
        </div>
      )}
      {showBootstrap && (
        <div className="w-full shrink-0">
          <BootstrapStepper phases={bootstrapPhases} />
        </div>
      )}
      {/* Above the Graph|Timeline toggle so the verdict's evidence is in
          the same place whichever view is selected: it is a property of
          the run, not of one rendering of it. */}
      <HarnessGateTable
        baseline={harnessBaseline}
        evidence={harnessEvidence}
        open={harnessOpen}
        onOpenChange={onHarnessOpenChange}
      />
    </div>
  );
}
