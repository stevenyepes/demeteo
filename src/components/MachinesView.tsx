import React, { useEffect, useRef, useState } from 'react';
import { Plus, Server, Key, Lock, Cpu, Edit2, Trash2, Wifi, WifiOff, Loader, AlertCircle, RefreshCw, Monitor, Rocket, CheckCircle2, X } from 'lucide-react';
import EnvModal, { blankForm, type EnvFormState } from './EnvModal';
import { formatError } from '../lib/errors';
import { useTauriEvent } from '../hooks/useTauriEvent';
import { useAgentCatalog, agentLabel } from '../lib/agentCatalog';
import {
  deleteMachine,
  deleteMachineSecret,
  listMachines,
  parseMachineAgents,
  testMachineConnection,
} from '../lib/machines';
import {
  cancelRunnerDownload,
  checkLocalRunner,
  downloadRunner,
  enableRemoteRuns,
  getRunnerStatus,
  type LocalRunnerCheck,
} from '../lib/runner';
import type { Machine } from '../types';

interface MachinesViewProps {
  /** Optional callback fired when a machine is added/updated/deleted,
   *  so parent screens (e.g. NewProjectView) can refresh their cache. */
  onChange?: () => void;
}

/**
 * Machines settings screen.
 *
 * Always shows a pinned built-in Local Machine card at the top.
 * Remote SSH machines are listed below it and can be added/edited/deleted.
 */
const MachinesView: React.FC<MachinesViewProps> = ({ onChange }) => {
  const { agents: agentCatalog } = useAgentCatalog();
  const [machines, setMachines] = useState<Machine[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>('');

  const [editing, setEditing] = useState<EnvFormState | null>(null);
  const [testState, setTestState] = useState<Record<string, 'idle' | 'testing' | 'ok' | 'err'>>({});
  const [testErrors, setTestErrors] = useState<Record<string, string>>({});

  // Remote runner install (docs/REMOTE_EXECUTION.md M7.1) — the
  // status pill below is auto-probed on mount + after every enable, so
  // every machine card shows "running / installed-stopped / not yet
  // installed" the moment the view opens, without the user having to
  // click anything. The "refresh" button (the small icon to the right
  // of the card) still re-probes on demand.
  const [runnerState, setRunnerState] = useState<Record<string, {
    status: 'idle' | 'checking' | 'downloading' | 'installing';
    version?: string | null;
    expectedVersion?: string | null;
    serviceActive?: boolean | null;
    lingering?: boolean | null;
    error?: string | null;
    downloadedBytes?: number;
    totalBytes?: number | null;
  }>>({});

  // Only one download is ever in flight (the version-keyed cache means
  // every machine reuses it), so a single ref is enough to route
  // progress events to the machine row that's actually waiting on it.
  const downloadingMachineIdRef = useRef<string | null>(null);
  useTauriEvent<{ downloaded: number; total: number | null }>('runner-download-progress', (p) => {
    const id = downloadingMachineIdRef.current;
    if (!id) return;
    setRunnerState((s) => ({
      ...s,
      [id]: { ...s[id], status: 'downloading', downloadedBytes: p.downloaded, totalBytes: p.total },
    }));
  });

  // Ambient, no-network/no-SSH info about what this laptop can push right
  // now — populated on mount so machines don't all look unknown until the
  // user clicks something, without auto-probing any remote host over SSH.
  const [localRunnerInfo, setLocalRunnerInfo] = useState<LocalRunnerCheck | null>(null);

  useEffect(() => {
    checkLocalRunner().then(setLocalRunnerInfo).catch(() => {});
  }, []);

  const probeRunner = async (m: Machine) => {
    setRunnerState((s) => ({ ...s, [m.id]: { ...s[m.id], status: 'checking' } }));
    try {
      const [result, local] = await Promise.all([getRunnerStatus(m.id), checkLocalRunner()]);
      setRunnerState((s) => ({
        ...s,
        [m.id]: {
          ...s[m.id],
          status: 'idle',
          version: result.installed ? result.version : null,
          serviceActive: result.service_active,
          lingering: result.lingering,
          expectedVersion: local.expected,
          error: null,
        },
      }));
    } catch (e) {
      setRunnerState((s) => ({ ...s, [m.id]: { ...s[m.id], status: 'idle', error: formatError(e) } }));
    }
  };

  const probeRunnerSilent = async (machineId: string) => {
    setRunnerState((s) => ({ ...s, [machineId]: { ...s[machineId], status: 'checking' } }));
    try {
      const result = await getRunnerStatus(machineId);
      setRunnerState((s) => ({
        ...s,
        [machineId]: {
          ...s[machineId],
          status: 'idle',
          version: result.installed ? result.version : null,
          serviceActive: result.service_active,
          lingering: result.lingering,
          error: null,
        },
      }));
    } catch (e) {
      setRunnerState((s) => ({
        ...s,
        [machineId]: { ...s[machineId], status: 'idle', error: formatError(e) },
      }));
    }
  };

  const enableRunner = async (m: Machine) => {
    try {
      const check = await checkLocalRunner();
      setLocalRunnerInfo(check);
      let localBinPath: string;
      if (check.status === 'ready') {
        if (check.stale_warning && !confirm(`Warning: ${check.stale_warning}. Push it to "${m.name}" anyway?`)) {
          return;
        }
        localBinPath = check.path;
      } else {
        if (!confirm(`demeteo-runner ${check.expected} is required but wasn't found locally. Download it now?`)) {
          return;
        }
        setRunnerState((s) => ({ ...s, [m.id]: { status: 'downloading', expectedVersion: check.expected } }));
        downloadingMachineIdRef.current = m.id;
        try {
          const downloaded = await downloadRunner();
          localBinPath = downloaded.path;
        } finally {
          downloadingMachineIdRef.current = null;
        }
      }
      setRunnerState((s) => ({ ...s, [m.id]: { status: 'installing', expectedVersion: check.expected } }));
      const result = await enableRemoteRuns(m.id, localBinPath);
      setRunnerState((s) => ({
        ...s,
        [m.id]: {
          ...s[m.id],
          status: 'idle',
          version: result.version,
          lingering: result.linger_enabled,
          serviceActive: true,
        },
      }));
    } catch (e) {
      setRunnerState((s) => ({ ...s, [m.id]: { status: 'idle', error: formatError(e) } }));
    }
  };

  const cancelDownload = async () => {
    try {
      await cancelRunnerDownload();
    } catch {
      // best-effort — the in-flight download call will surface its own
      // "cancelled" error to the catch block in enableRunner either way
    }
  };

  const fetchMachines = async () => {
    setLoading(true);
    setError('');
    try {
      const list = await listMachines();
      setMachines(list ?? []);
      list.forEach((m) => { void probeRunnerSilent(m.id); });
    } catch (e) {
      setError(formatError(e));
      setMachines([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchMachines();
  }, []);

  const machineToForm = (m: Machine): EnvFormState => {
    // Parse the persisted agents JSON into the UI's kind list. EnvModal keys
    // its selectable-agent buttons off the registry catalog by kind and
    // renders the display label itself, so we store kinds here (not labels).
    const agentNames = parseMachineAgents(m.agents)
      .filter((a) => a.enabled !== false)
      .map((a) => a.kind ?? '')
      .filter((k) => k);
    const connection = m.username
      ? `${m.username}@${m.host}${m.port && m.port !== 22 ? `:${m.port}` : ''}`
      : `${m.host}${m.port && m.port !== 22 ? `:${m.port}` : ''}`;
    return {
      id: m.id,
      name: m.name,
      connection,
      authType: m.auth_type,
      keyPath: m.key_path ?? '',
      secret: '', // never pre-fill the passphrase; user re-enters to change
      agents: agentNames,
      useLoginShell: m.use_login_shell ?? false,
      setupCommands: (() => {
        if (!m.setup_commands) return '';
        try {
          const arr = JSON.parse(m.setup_commands);
          return Array.isArray(arr) ? arr.join('\n') : '';
        } catch { return ''; }
      })(),
      notifyWebhookUrl: m.notify_webhook_url ?? '',
    };
  };

  const handleAdd = () => {
    setEditing({ ...blankForm, authType: 'key', connection: '' });
  };

  const handleEdit = (m: Machine) => {
    setEditing(machineToForm(m));
  };

  const handleSaved = () => {
    fetchMachines();
    onChange?.();
  };

  const handleDeleted = () => {
    fetchMachines();
    onChange?.();
  };

  const handleTest = async (m: Machine) => {
    setTestState((s) => ({ ...s, [m.id]: 'testing' }));
    setTestErrors((s) => ({ ...s, [m.id]: '' }));
    try {
      await testMachineConnection(m.id);
      setTestState((s) => ({ ...s, [m.id]: 'ok' }));
    } catch (e) {
      setTestState((s) => ({ ...s, [m.id]: 'err' }));
      setTestErrors((s) => ({ ...s, [m.id]: formatError(e) }));
    }
  };

  const handleQuickDelete = async (m: Machine) => {
    if (!confirm(`Delete machine "${m.name}"? This removes its stored credentials.`)) return;
    try {
      await deleteMachine(m.id);
      try { await deleteMachineSecret(m.id); } catch { /* ok */ }
      fetchMachines();
      onChange?.();
    } catch (e) {
      setError(formatError(e));
    }
  };

  return (
    <div className="flex-1 overflow-y-auto p-8 relative">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-cyan-600/5 rounded-full blur-[120px] pointer-events-none"></div>

      <div className="max-w-4xl mx-auto relative z-10">
        <div className="flex items-end justify-between mb-6 border-b border-white/5 pb-4">
          <div>
            <h2 className="text-2xl font-outfit font-bold text-white mb-1">Machines</h2>
            <p className="text-sm text-slate-400">
              Environments that Demeteo can run agents on. The local machine is always available; add remote SSH hosts below.
            </p>
          </div>
          <div className="flex gap-2">
            <button
              onClick={fetchMachines}
              className="px-3 py-2 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5"
              title="Refresh"
            >
              <RefreshCw className="w-3.5 h-3.5" />
              Refresh
            </button>
            <button
              onClick={handleAdd}
              className="px-4 py-2 text-xs font-bold rounded-lg bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all flex items-center gap-1.5"
            >
              <Plus className="w-4 h-4" />
              Add Machine
            </button>
          </div>
        </div>

        {error && (
          <div className="mb-4 text-[12px] text-red-300 bg-red-500/10 border border-red-500/20 rounded-lg p-3 flex items-start gap-2">
            <AlertCircle className="w-4 h-4 mt-0.5 shrink-0" />
            <span className="break-all">{error}</span>
          </div>
        )}

        {localRunnerInfo && (
          <div className="mb-4 text-[11px] text-slate-400 flex items-center gap-1.5">
            {localRunnerInfo.status === 'ready' ? (
              <>
                <CheckCircle2 className="w-3 h-3 text-emerald-400 shrink-0" />
                {localRunnerInfo.stale_warning
                  ? `This laptop has a demeteo-runner build, but ${localRunnerInfo.stale_warning}.`
                  : `This laptop has demeteo-runner ${localRunnerInfo.version ?? localRunnerInfo.expected} ready to push to any machine.`}
              </>
            ) : (
              <>
                <AlertCircle className="w-3 h-3 text-slate-500 shrink-0" />
                {`demeteo-runner ${localRunnerInfo.expected} isn't cached on this laptop yet — "Enable remote runs" will prompt to download it.`}
              </>
            )}
          </div>
        )}

        {/* Built-in local machine — always shown, non-editable */}
        <div className="glass-panel p-4 flex items-start justify-between gap-4 border-l-2 border-l-emerald-500/60 mb-3">
          <div className="flex items-start gap-3 min-w-0">
            <div className="w-9 h-9 rounded-lg bg-gradient-to-br from-emerald-500/20 to-cyan-500/20 border border-white/10 flex items-center justify-center shrink-0">
              <Monitor className="w-4 h-4 text-emerald-400" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h4 className="text-base font-semibold text-white font-outfit">This Machine</h4>
                <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">Built-in</span>
              </div>
              <div className="text-xs text-slate-400 mt-1 font-mono">
                <p>Runs agents directly on this computer via a local PTY</p>
              </div>
            </div>
          </div>
        </div>

        {/* Remote SSH machines */}
        {loading ? (
          <div className="text-center py-8 text-slate-500 text-sm">
            <Loader className="w-5 h-5 animate-spin mx-auto mb-2 text-cyan-400" />
            Loading machines…
          </div>
        ) : machines.length === 0 ? (
          <div className="glass-panel p-8 text-center flex flex-col items-center justify-center">
            <Server className="w-8 h-8 text-slate-500 mb-3" />
            <h3 className="text-base font-outfit font-semibold text-white mb-1">No remote machines</h3>
            <p className="text-sm text-slate-400 max-w-md mb-5">
              Add a remote SSH host to run agents on cloud instances or other servers.
            </p>
            <button
              onClick={handleAdd}
              className="px-5 py-2.5 text-sm font-bold rounded-lg bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all flex items-center gap-1.5"
            >
              <Plus className="w-4 h-4" />
              Add remote machine
            </button>
          </div>
        ) : (
          <div className="space-y-3">
            {machines.map((m) => {
              const conn = testState[m.id] ?? 'idle';
              const agents = parseMachineAgents(m.agents)
                .filter((a) => a.enabled !== false)
                .map((a) => agentLabel(agentCatalog, a.kind ?? '?'));
              const authLabel =
                m.auth_type === 'key' ? 'Private Key' :
                m.auth_type === 'password' ? 'Password' :
                m.auth_type === 'agent' ? 'SSH Agent' : m.auth_type;
              return (
                <div
                  key={m.id}
                  className="glass-panel p-4 flex items-start justify-between gap-4 border-l-2 border-l-cyan-500/60"
                >
                  <div className="flex items-start gap-3 min-w-0">
                    <div className="w-9 h-9 rounded-lg bg-gradient-to-br from-violet-500/20 to-cyan-500/20 border border-white/10 flex items-center justify-center shrink-0">
                      {m.auth_type === 'key' ? (
                        <Key className="w-4 h-4 text-cyan-400" />
                      ) : m.auth_type === 'password' ? (
                        <Lock className="w-4 h-4 text-violet-400" />
                      ) : (
                        <Server className="w-4 h-4 text-emerald-400" />
                      )}
                    </div>
                    <div className="min-w-0">
                      <h4 className="text-base font-semibold text-white font-outfit truncate">{m.name}</h4>
                      <div className="text-xs text-slate-400 mt-1 space-y-0.5 font-mono">
                        <p>
                          {m.username ? <><span className="text-slate-200">{m.username}</span>@</> : null}
                          <span className="text-slate-200">{m.host}</span>
                          {m.port && m.port !== 22 ? <span className="text-slate-200">:{m.port}</span> : null}
                        </p>
                        <p className="flex flex-wrap gap-x-3 gap-y-0.5">
                          <span>Auth: <span className="text-slate-200">{authLabel}</span></span>
                          {m.key_path && (
                            <span className="truncate" title={m.key_path}>
                              Key: <span className="text-slate-200">{m.key_path}</span>
                            </span>
                          )}
                        </p>
                        {agents.length > 0 && (
                          <p className="flex items-center gap-1.5">
                            <Cpu className="w-3 h-3" />
                            <span>Agents: <span className="text-slate-200">{agents.join(', ')}</span></span>
                          </p>
                        )}
                      </div>
                      {conn === 'ok' && (
                        <p className="mt-2 text-[11px] text-emerald-400 flex items-center gap-1">
                          <Wifi className="w-3 h-3" /> Connection OK
                        </p>
                      )}
                      {conn === 'err' && (
                        <p className="mt-2 text-[11px] text-red-400 flex items-start gap-1">
                          <WifiOff className="w-3 h-3 mt-0.5 shrink-0" />
                          <span className="break-all">{testErrors[m.id]}</span>
                        </p>
                      )}
                      {runnerState[m.id]?.status === 'downloading' && (
                        <p className="mt-2 text-[11px] text-cyan-300 flex items-center gap-1">
                          <Loader className="w-3 h-3 animate-spin" />
                          Downloading demeteo-runner {runnerState[m.id]?.expectedVersion}
                          {(() => {
                            const st = runnerState[m.id];
                            if (!st?.downloadedBytes) return '…';
                            const mb = (n: number) => `${(n / (1024 * 1024)).toFixed(1)}MB`;
                            return st.totalBytes
                              ? ` — ${mb(st.downloadedBytes)} / ${mb(st.totalBytes)} (${Math.round((st.downloadedBytes / st.totalBytes) * 100)}%)`
                              : ` — ${mb(st.downloadedBytes)}`;
                          })()}
                        </p>
                      )}
                      {runnerState[m.id]?.status === 'installing' && (
                        <p className="mt-2 text-[11px] text-cyan-300 flex items-center gap-1">
                          <Loader className="w-3 h-3 animate-spin" />
                          Installing demeteo-runner {runnerState[m.id]?.expectedVersion} on {m.name}…
                        </p>
                      )}
                      {(() => {
                        const st = runnerState[m.id];
                        if (!st || st.status === 'downloading' || st.status === 'installing') return null;
                        if (st.error) {
                          return (
                            <p className="mt-2 text-[11px] text-red-400 flex items-start gap-1">
                              <AlertCircle className="w-3 h-3 mt-0.5 shrink-0" />
                              <span className="break-all">{st.error}</span>
                            </p>
                          );
                        }
                        if (st.status === 'checking' && !st.version) {
                          return (
                            <p className="mt-2 text-[11px] text-slate-500 flex items-center gap-1">
                              <Loader className="w-3 h-3 animate-spin" />
                              Checking demeteo-runner status…
                            </p>
                          );
                        }
                        if (!st.version) {
                          return (
                            <p className="mt-2 text-[11px] text-slate-500 flex items-center gap-1">
                              <Cpu className="w-3 h-3" />
                              Remote runner not installed — click <span className="text-slate-300">Enable remote runs</span> to provision it.
                            </p>
                          );
                        }
                        const v = st.version;
                        const stale = st.expectedVersion && v !== st.expectedVersion;
                        const isRunning = st.serviceActive === true;
                        const pillTone = isRunning
                          ? 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300'
                          : st.serviceActive === false
                          ? 'border-white/10 bg-white/5 text-slate-300'
                          : 'border-white/10 bg-white/5 text-slate-400';
                        const dotTone = isRunning
                          ? 'bg-emerald-400 shadow-[0_0_8px_rgba(16,185,129,0.7)]'
                          : st.serviceActive === false
                          ? 'bg-slate-500'
                          : 'bg-slate-600';
                        const label = isRunning
                          ? 'Running'
                          : st.serviceActive === false
                          ? 'Installed, stopped'
                          : 'Installed';
                        return (
                          <>
                            <div className={`mt-2 inline-flex items-center gap-1.5 px-2 py-1 rounded-md border text-[11px] font-medium max-w-full ${pillTone}`}>
                              <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${dotTone} ${isRunning ? 'animate-pulse' : ''}`} />
                              <span>{label}</span>
                              <span>·</span>
                              <span className="font-mono">{v}</span>
                              {stale && (
                                <span className="text-slate-400">
                                  · update available ({st.expectedVersion})
                                </span>
                              )}
                            </div>
                            {isRunning && st.lingering === false && (
                              <div className="mt-2 text-[11px] text-amber-300 bg-amber-500/10 border border-amber-500/20 rounded-lg p-2.5 flex items-start gap-2">
                                <AlertCircle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                                <span className="break-words">
                                  Lingering isn't enabled for this user — the runner will stop when you log out of SSH and won't auto-start on reboot. Ask an administrator to run <code className="px-1 py-0.5 rounded bg-white/5 text-amber-200">loginctl enable-linger &lt;user&gt;</code> on this machine.
                                </span>
                              </div>
                            )}
                            {st.serviceActive === false && (
                              <p className="mt-1 text-[11px] text-slate-400 flex items-start gap-1">
                                <AlertCircle className="w-3 h-3 mt-0.5 shrink-0" />
                                <span className="break-words">Service is installed but not running. Run <code className="px-1 py-0.5 rounded bg-white/5 text-slate-200">systemctl --user start demeteo-runner</code> on the remote host, or click <span className="text-slate-200">Upgrade runner</span> to re-provision.</span>
                              </p>
                            )}
                          </>
                        );
                      })()}
                    </div>
                  </div>

                  <div className="flex items-center gap-1.5 shrink-0">
                    <button
                      onClick={() => probeRunner(m)}
                      disabled={runnerState[m.id]?.status === 'checking'}
                      className="px-2 py-1.5 text-[11px] rounded-lg text-slate-500 hover:text-slate-200 hover:bg-white/5 transition-all disabled:opacity-50"
                      title="Check demeteo-runner status without installing"
                    >
                      {runnerState[m.id]?.status === 'checking' ? (
                        <Loader className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                      ) : (
                        <RefreshCw className="w-3.5 h-3.5" />
                      )}
                    </button>
                    <button
                      onClick={() => enableRunner(m)}
                      disabled={runnerState[m.id]?.status === 'installing' || runnerState[m.id]?.status === 'downloading'}
                      className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5 disabled:opacity-50"
                      title={runnerState[m.id]?.version ? 'Download (if needed), push the latest build and restart the runner' : 'Download and install demeteo-runner as a systemd --user service'}
                    >
                      {runnerState[m.id]?.status === 'installing' || runnerState[m.id]?.status === 'downloading' ? (
                        <Loader className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                      ) : (
                        <Rocket className="w-3.5 h-3.5" />
                      )}
                      {runnerState[m.id]?.status === 'downloading'
                        ? 'Downloading…'
                        : runnerState[m.id]?.version
                        ? 'Upgrade runner'
                        : 'Enable remote runs'}
                    </button>
                    {runnerState[m.id]?.status === 'downloading' && (
                      <button
                        onClick={cancelDownload}
                        className="px-2 py-1.5 text-[11px] rounded-lg text-slate-500 hover:text-red-400 hover:bg-red-500/10 transition-all"
                        title="Cancel download"
                      >
                        <X className="w-3.5 h-3.5" />
                      </button>
                    )}
                    <button
                      onClick={() => handleTest(m)}
                      disabled={conn === 'testing'}
                      className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5 disabled:opacity-50"
                      title="Test SSH connection"
                    >
                      {conn === 'testing' ? (
                        <Loader className="w-3.5 h-3.5 animate-spin text-cyan-400" />
                      ) : conn === 'ok' ? (
                        <Wifi className="w-3.5 h-3.5 text-emerald-400" />
                      ) : conn === 'err' ? (
                        <WifiOff className="w-3.5 h-3.5 text-red-400" />
                      ) : (
                        <Wifi className="w-3.5 h-3.5" />
                      )}
                      Test
                    </button>
                    <button
                      onClick={() => handleEdit(m)}
                      className="px-2.5 py-1.5 text-[11px] font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 flex items-center gap-1.5"
                      title="Edit machine"
                    >
                      <Edit2 className="w-3.5 h-3.5" />
                      Edit
                    </button>
                    <button
                      onClick={() => handleQuickDelete(m)}
                      className="px-2 py-1.5 text-[11px] rounded-lg text-slate-500 hover:text-red-400 hover:bg-red-500/10 transition-all"
                      title="Delete machine"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {editing && (
        <EnvModal
          isOpen={true}
          initialData={editing}
          onClose={() => setEditing(null)}
          onSaved={handleSaved}
          onDeleted={handleDeleted}
        />
      )}
    </div>
  );
};

export default MachinesView;