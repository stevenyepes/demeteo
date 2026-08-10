import type { HarnessBaseline, RemoteRunMirror, RunEvent } from '../../types';
import type { HarnessEvidence } from '../../lib/harnessVerdict';
import { TERMINAL_STATUSES } from '../../lib/runStatus';
import { relativeTime } from '../../lib/utils';
import { BootstrapStepper, type BootstrapPhaseView } from '../BootstrapStepper';
import { HarnessGateTable } from '../HarnessGateTable';
import { RemoteGateActions, ReinjectCredentials } from '../RemoteRunActions';
import { bucketFor } from '../RemoteRunInbox';
import type { RunLayoutMode } from '../runLayout';
import { ActivityPanel } from './ActivityPanel';

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
}

/**
 * At 'split' the meta panels take their own track, so a wide window gains a
 * second column instead of a longer scroll — chronology still reads
 * top-to-bottom and prose keeps its measure. At 'stacked' they fall back
 * above the graph, which is the only case where they count as its chrome —
 * hence the conditional ref.
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
}: RunMetaColumnProps) {
  const remoteTerminal = remoteRun !== null && TERMINAL_STATUSES.includes(remoteRun.status);
  // A local run with nothing in its feed yet gets no panel at all: the push
  // starts on mount and is never backfilled, so an empty Activity block on a
  // finished feature would be a permanent, unexplained blank.
  const showActivity = remoteRun !== null || runEvents.length > 0;

  return (
    <div
      ref={runLayout === 'split' ? undefined : setMetaChromeEl}
      // Split, this is one of three full-height tracks and scrolls itself. The
      // width arrives as a number rather than a class because `runLayout.ts`
      // has to subtract it to size the pane pair beside it, and a share spelled
      // once in CSS and once in TypeScript is two answers waiting to disagree.
      style={widthPx === null ? undefined : { width: widthPx }}
      className={`flex shrink-0 flex-col ${
        runLayout === 'split'
          ? 'h-full min-h-0 overflow-y-auto overflow-x-hidden'
          : 'w-full min-w-0'
      }`}
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
              {bucketFor(remoteRun.status) === 'parked' && (
                <RemoteGateActions run={remoteRun} onResolved={onRemoteResolved} />
              )}
              {bucketFor(remoteRun.status) === 'needs_credentials' && (
                <ReinjectCredentials run={remoteRun} onResolved={onRemoteResolved} />
              )}
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
      <HarnessGateTable baseline={harnessBaseline} evidence={harnessEvidence} />
    </div>
  );
}
