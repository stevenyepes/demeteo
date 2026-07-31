import type { HarnessBaseline, RemoteRunMirror, RunEvent } from '../../types';
import type { HarnessEvidence } from '../../lib/harnessVerdict';
import { TERMINAL_STATUSES } from '../../lib/runStatus';
import { relativeTime } from '../../lib/utils';
import { BootstrapStepper, type BootstrapPhaseView } from '../BootstrapStepper';
import { HarnessGateTable } from '../HarnessGateTable';
import { RemoteGateActions, ReinjectCredentials, RunEventTimeline } from '../RunEventTimeline';
import { bucketFor } from '../RemoteRunInbox';
import type { RunLayoutMode } from '../runLayout';

interface RunMetaColumnProps {
  runLayout: RunLayoutMode;
  setMetaChromeEl: (el: HTMLDivElement | null) => void;
  remoteRun: RemoteRunMirror | null;
  remoteMachineName: string | null;
  onRunEvents: (events: RunEvent[]) => void;
  onRemoteResolved: () => void;
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
  setMetaChromeEl,
  remoteRun,
  remoteMachineName,
  onRunEvents,
  onRemoteResolved,
  showBootstrap,
  bootstrapPhases,
  harnessBaseline,
  harnessEvidence,
}: RunMetaColumnProps) {
  return (
    <div
      ref={runLayout === 'split' ? undefined : setMetaChromeEl}
      className={`flex shrink-0 flex-col ${runLayout === 'split' ? 'w-[26rem]' : 'w-full min-w-0'}`}
    >
      {remoteRun && (
        /* Activity feed for a detached run: the runner's own event
           log (submitted → cloned → gates → pushed → PR), inline
           where the run lives instead of a separate modal. */
        <div className="mb-6 w-full shrink-0 space-y-1.5">
          <RunEventTimeline
            run={remoteRun}
            machineName={remoteMachineName ?? remoteRun.machine_id}
            onEvents={onRunEvents}
          />
          <div className="flex items-center justify-between gap-3 px-1">
            <p className="text-[10px] font-mono text-slate-500">
              {TERMINAL_STATUSES.includes(remoteRun.status)
                ? `Final state synced ${relativeTime(remoteRun.updated_at)}`
                : `Last synced ${relativeTime(remoteRun.updated_at)} · polling every 3s`}
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
