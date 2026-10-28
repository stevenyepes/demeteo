import { useCallback, useEffect, useRef, useState } from 'react';
import { AlertTriangle, RotateCw } from 'lucide-react';
import {
  hubConnection,
  hubDisconnect,
  hubMachine,
  hubPair,
  hubPolicyGet,
  hubPolicySet,
  hubRequestLog,
  hubStatus,
  hubTrustedPasskeys,
  type HubConnection,
  type HubLocalPolicy,
  type HubMachine,
  type HubRequestLogEntry,
  type HubStatus,
  type HubTrustedPasskey,
} from '../../lib/hub';
import { reportError } from '../../lib/errorBus';
import { HubActivityLog } from './HubActivityLog';
import { HubCeilingsCard, type CeilingField } from './HubCeilingsCard';
import { HubConnectionCard } from './HubConnectionCard';
import { HubDisconnectCard } from './HubDisconnectCard';
import { HubPasskeysCard } from './HubPasskeysCard';
import { HubScopesCard, type ScopeField } from './HubScopesCard';

/** Spec D5: the Hub's connection and activity log are read by polling, not
 *  by a domain event. */
const POLL_MS = 5000;

const ACTIVITY_LIMIT = 50;

type HubView =
  | { kind: 'loading' }
  | { kind: 'unavailable'; message: string }
  | { kind: 'ready'; status: HubStatus };

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function HubTab() {
  const [view, setView] = useState<HubView>({ kind: 'loading' });
  const [connection, setConnection] = useState<HubConnection | null>(null);
  const [machine, setMachine] = useState<HubMachine | null>(null);
  const [pairing, setPairing] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [policy, setPolicy] = useState<HubLocalPolicy | null>(null);
  const [savingPolicy, setSavingPolicy] = useState(false);
  const savingRef = useRef(false);
  const [passkeys, setPasskeys] = useState<HubTrustedPasskey[]>([]);
  const [activity, setActivity] = useState<HubRequestLogEntry[]>([]);
  const available = view.kind === 'ready';

  useEffect(() => {
    let cancelled = false;
    hubPolicyGet().then(
      (next) => {
        if (!cancelled) setPolicy(next);
      },
      (e) => {
        if (!cancelled) reportError(e);
      },
    );
    hubTrustedPasskeys().then(
      (next) => {
        if (!cancelled) setPasskeys(next);
      },
      (e) => {
        if (!cancelled) reportError(e);
      },
    );
    hubRequestLog(ACTIVITY_LIMIT).then(
      (next) => {
        if (!cancelled) setActivity(next);
      },
      (e) => {
        if (!cancelled) reportError(e);
      },
    );
    const load = async () => {
      let status: HubStatus;
      try {
        status = await hubStatus();
      } catch (e) {
        if (!cancelled) setView({ kind: 'unavailable', message: messageOf(e) });
        return;
      }
      if (cancelled) return;
      setView({ kind: 'ready', status });
      try {
        const [nextConnection, nextMachine] = await Promise.all([hubConnection(), hubMachine()]);
        if (cancelled) return;
        setConnection(nextConnection);
        setMachine(nextMachine);
      } catch (e) {
        if (!cancelled) reportError(e);
      }
    };
    load();
    return () => {
      cancelled = true;
    };
  }, []);

  const refreshConnection = useCallback(async () => {
    try {
      setConnection(await hubConnection());
    } catch {
      setConnection(null);
    }
  }, []);

  const refreshActivity = useCallback(async () => {
    try {
      setActivity(await hubRequestLog(ACTIVITY_LIMIT));
    } catch {
      // The first load reported it; a failed poll keeps the rows already shown.
    }
  }, []);

  useEffect(() => {
    const timer = setInterval(() => {
      refreshActivity();
      if (available) refreshConnection();
    }, POLL_MS);
    return () => clearInterval(timer);
  }, [available, refreshActivity, refreshConnection]);

  const handlePair = async (hubUrl: string) => {
    setPairing(true);
    try {
      const status = await hubPair(hubUrl);
      setView({ kind: 'ready', status });
      await refreshConnection();
      setPasskeys(await hubTrustedPasskeys());
    } catch (e) {
      reportError(e);
    } finally {
      setPairing(false);
    }
  };

  const handleDisconnect = async () => {
    setDisconnecting(true);
    try {
      const status = await hubDisconnect();
      setView({ kind: 'ready', status });
      await refreshConnection();
      setPasskeys(await hubTrustedPasskeys());
    } catch (e) {
      reportError(e);
    } finally {
      setDisconnecting(false);
    }
  };

  /** One `hub_policy_set` at a time: each save sends the whole policy, and
   *  Tauri runs two in-flight commands concurrently, so a second save would
   *  carry the first's optimistic value and land in either order. The ref
   *  guards against a click that lands before the disabling re-render. */
  const savePolicy = async <F extends keyof HubLocalPolicy>(field: F, value: HubLocalPolicy[F]) => {
    if (!policy || savingRef.current) return;
    savingRef.current = true;
    setSavingPolicy(true);
    const was = policy[field];
    setPolicy((current) => current && { ...current, [field]: value });
    try {
      await hubPolicySet({ ...policy, [field]: value });
    } catch (e) {
      try {
        setPolicy(await hubPolicyGet());
      } catch {
        setPolicy((current) => current && { ...current, [field]: was });
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

  const localCards = (
    <>
      <HubScopesCard policy={policy} saving={savingPolicy} onToggle={handleToggle} />
      <HubCeilingsCard policy={policy} saving={savingPolicy} onSave={handleCeiling} />
      <HubPasskeysCard passkeys={passkeys} />
      <HubActivityLog entries={activity} />
    </>
  );

  if (view.kind === 'loading') {
    return (
      <div className="flex items-center justify-center py-8">
        <RotateCw className="w-5 h-5 text-cyan-400 animate-spin" />
      </div>
    );
  }

  if (view.kind === 'unavailable') {
    return (
      <div className="space-y-4">
        <div className="glass-panel p-6 rounded-xl space-y-3">
          <h3 className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
            <AlertTriangle className="w-4 h-4 text-amber-400" /> Hub unavailable
          </h3>
          <p className="text-xs text-slate-400 leading-relaxed">{view.message}</p>
        </div>
        {localCards}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <HubConnectionCard
        status={view.status}
        connection={connection}
        machine={machine}
        pairing={pairing}
        onPair={handlePair}
      />
      {localCards}
      {view.status.state !== 'unpaired' && (
        <HubDisconnectCard disconnecting={disconnecting} onDisconnect={handleDisconnect} />
      )}
    </div>
  );
}

export default HubTab;
