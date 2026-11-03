import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { AlertTriangle, ChevronRight, Cloud, RefreshCw, RotateCw } from 'lucide-react';
import type { HubLocalPolicy, HubRequestLogEntry } from '../lib/hub';
import {
  remoteHubDisconnect,
  remoteHubPairBegin,
  remoteHubPairCancel,
  remoteHubPairState,
  remoteHubPolicyGet,
  remoteHubPolicySet,
  remoteHubRequestLog,
  remoteHubStatus,
  type RunnerHubStatus,
  type RunnerPairState,
} from '../lib/runnerHub';
import { formatError } from '../lib/errors';
import { reportError } from '../lib/errorBus';
import { HubActivityLog } from './settings/HubActivityLog';
import { HubCeilingsCard, type CeilingField } from './settings/HubCeilingsCard';
import { HubDisconnectCard } from './settings/HubDisconnectCard';
import { HubScopesCard, type ScopeField } from './settings/HubScopesCard';
import { RunnerHubPairingCard } from './RunnerHubPairingCard';
import { Button } from './ui/Button';

/** How often a pending pairing is asked whether the Hub approved it. */
export const PAIR_POLL_MS = 2000;

const ACTIVITY_LIMIT = 50;

type HubView =
  | { kind: 'loading' }
  | { kind: 'too_old' }
  | { kind: 'unavailable'; message: string }
  | { kind: 'ready'; hub: RunnerHubStatus };

/** `null` entries: the runner predates its activity log. */
type Activity = HubRequestLogEntry[] | null;

interface RunnerHubPanelProps {
  machineId: string;
}

/**
 * A remote runner's Hub pairing, scopes, ceilings and activity, every one
 * read from and written to the runner over SSH. Collapsed until asked for,
 * and the body unmounts when closed: each machine card mounts one, and none
 * should reach its runner until the user looks.
 */
export function RunnerHubPanel({ machineId }: RunnerHubPanelProps) {
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  return (
    <section className="space-y-3">
      <Button
        variant="ghost"
        size="sm"
        aria-expanded={open}
        aria-controls={open ? bodyId : undefined}
        onClick={() => setOpen((was) => !was)}
      >
        <ChevronRight
          aria-hidden="true"
          className={`w-3.5 h-3.5 transition-transform ${open ? 'rotate-90' : ''}`}
        />
        <Cloud className="w-3.5 h-3.5" /> Hub pairing
      </Button>
      {open && (
        <div id={bodyId}>
          <RunnerHubBody machineId={machineId} />
        </div>
      )}
    </section>
  );
}

function RunnerHubBody({ machineId }: RunnerHubPanelProps) {
  const [view, setView] = useState<HubView>({ kind: 'loading' });
  const [pairState, setPairState] = useState<RunnerPairState>({ state: 'idle' });
  const [pairError, setPairError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [policy, setPolicy] = useState<HubLocalPolicy | null>(null);
  const [savingPolicy, setSavingPolicy] = useState(false);
  const savingRef = useRef(false);
  const [activity, setActivity] = useState<Activity>([]);

  const load = useCallback(async () => {
    let hub: RunnerHubStatus | null;
    try {
      hub = await remoteHubStatus(machineId);
    } catch (e) {
      setView({ kind: 'unavailable', message: formatError(e) });
      return;
    }
    if (hub === null) {
      setView({ kind: 'too_old' });
      return;
    }
    setView({ kind: 'ready', hub });
    const [nextPair, nextPolicy, nextActivity] = await Promise.allSettled([
      remoteHubPairState(machineId),
      remoteHubPolicyGet(machineId),
      remoteHubRequestLog(machineId, ACTIVITY_LIMIT),
    ]);
    if (nextPair.status === 'fulfilled') setPairState(nextPair.value);
    if (nextPolicy.status === 'fulfilled') setPolicy(nextPolicy.value);
    else reportError(nextPolicy.reason);
    if (nextActivity.status === 'fulfilled') setActivity(nextActivity.value);
    else reportError(nextActivity.reason);
  }, [machineId]);

  useEffect(() => {
    load();
  }, [load]);

  const pending = pairState.state === 'pending';

  useEffect(() => {
    if (!pending) return;
    const timer = setInterval(async () => {
      let next: RunnerPairState;
      try {
        next = await remoteHubPairState(machineId);
      } catch {
        // An unreachable runner keeps the code on screen; the next tick asks again.
        return;
      }
      setPairState(next);
      if (next.state === 'idle') load();
    }, PAIR_POLL_MS);
    return () => clearInterval(timer);
  }, [pending, machineId, load]);

  const handlePair = async (hubUrl: string) => {
    setBusy(true);
    setPairError(null);
    try {
      const ticket = await remoteHubPairBegin(machineId, hubUrl);
      setPairState({ state: 'pending', ticket });
    } catch (e) {
      setPairError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const handleCancel = async () => {
    setBusy(true);
    try {
      setPairState(await remoteHubPairCancel(machineId));
    } catch (e) {
      reportError(e);
    } finally {
      setBusy(false);
    }
  };

  const handleDisconnect = async () => {
    setDisconnecting(true);
    try {
      await remoteHubDisconnect(machineId);
      await load();
    } catch (e) {
      reportError(e);
    } finally {
      setDisconnecting(false);
    }
  };

  /** One `hub_policy_set` at a time, for the reason `HubTab`'s `savePolicy`
   *  gives: each save sends the whole policy. */
  const savePolicy = async <F extends keyof HubLocalPolicy>(field: F, value: HubLocalPolicy[F]) => {
    if (!policy || savingRef.current) return;
    savingRef.current = true;
    setSavingPolicy(true);
    setPolicy({ ...policy, [field]: value });
    try {
      setPolicy(await remoteHubPolicySet(machineId, { ...policy, [field]: value }));
    } catch (e) {
      try {
        setPolicy(await remoteHubPolicyGet(machineId));
      } catch {
        setPolicy(policy);
      }
      reportError(e);
    } finally {
      savingRef.current = false;
      setSavingPolicy(false);
    }
  };

  const handleToggle = (field: ScopeField) => {
    if (policy) savePolicy(field, !policy[field]);
  };

  const handleCeiling = (field: CeilingField, cents: number | null) => savePolicy(field, cents);

  if (view.kind === 'loading') {
    return (
      <div className="flex items-center justify-center py-6">
        <RotateCw className="w-5 h-5 text-cyan-400 animate-spin" aria-label="Loading" />
      </div>
    );
  }

  if (view.kind === 'too_old' || view.kind === 'unavailable') {
    return (
      <div className="panel p-6 space-y-3">
        <h3 className="font-heading text-sm font-semibold text-slate-300 flex items-center gap-2">
          <AlertTriangle className="w-4 h-4 text-slate-400" />
          {view.kind === 'too_old' ? 'This runner predates Hub pairing' : 'Hub pairing unavailable'}
        </h3>
        <p className="text-xs text-slate-400 leading-relaxed break-words">
          {view.kind === 'too_old'
            ? 'Upgrade the runner to pair it with a Hub.'
            : view.message}
        </p>
        <Button size="sm" onClick={() => load()}>
          <RefreshCw className="w-3.5 h-3.5" /> Try again
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <RunnerHubPairingCard
        hub={view.hub}
        pairState={pairState}
        pairError={pairError}
        busy={busy}
        onPair={handlePair}
        onCancel={handleCancel}
      />
      <HubScopesCard policy={policy} saving={savingPolicy} onToggle={handleToggle} />
      <HubCeilingsCard policy={policy} saving={savingPolicy} onSave={handleCeiling} />
      {activity === null ? (
        <div className="panel p-6">
          <p className="text-xs text-slate-400">
            This runner is too old to report its Hub activity. Upgrade it to see what the Hub asked.
          </p>
        </div>
      ) : (
        <HubActivityLog
          entries={activity}
          description="Every request the Hub sent this runner, newest first, and what the runner did with it."
        />
      )}
      {view.hub.status.state !== 'unpaired' && (
        <HubDisconnectCard disconnecting={disconnecting} onDisconnect={handleDisconnect} />
      )}
    </div>
  );
}

export default RunnerHubPanel;
