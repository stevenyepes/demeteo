import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Server, Key, Lock, Cpu, Edit2, Trash2, Wifi, WifiOff, Loader, AlertCircle, RefreshCw } from 'lucide-react';
import EnvModal, { blankForm, type EnvFormState } from './EnvModal';

interface Machine {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: string;
  key_path?: string | null;
  agents?: string | null;
  use_login_shell?: boolean | null;
  setup_commands?: string | null;
}

interface MachinesViewProps {
  /** Optional callback fired when a machine is added/updated/deleted,
   *  so parent screens (e.g. NewProjectView) can refresh their cache. */
  onChange?: () => void;
}

/**
 * Machines (SSH environments) settings screen.
 *
 * - Lists every machine in the SQLite `machines` table.
 * - "Add machine" / per-row "Edit" open the existing `EnvModal`
 *   (which already had the passphrase field — it was just never
 *   wired into the UI).
 * - Per-row "Test connection" runs the existing
 *   `test_machine_connection` Tauri command.
 * - Delete wipes both the row and the keyring-stored secret via
 *   `delete_machine` + `delete_machine_secret`.
 */
const MachinesView: React.FC<MachinesViewProps> = ({ onChange }) => {
  const [machines, setMachines] = useState<Machine[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>('');

  const [editing, setEditing] = useState<EnvFormState | null>(null);
  const [testState, setTestState] = useState<Record<string, 'idle' | 'testing' | 'ok' | 'err'>>({});
  const [testErrors, setTestErrors] = useState<Record<string, string>>({});

  const fetchMachines = async () => {
    setLoading(true);
    setError('');
    try {
      const list: Machine[] = await invoke('get_machines');
      setMachines(list ?? []);
    } catch (e: any) {
      setError(String(e));
      setMachines([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchMachines();
  }, []);

  const machineToForm = (m: Machine): EnvFormState => {
    // Parse the persisted agents JSON back into the UI's display names.
    let agentNames: string[] = [];
    try {
      const arr = m.agents ? JSON.parse(m.agents) : [];
      if (Array.isArray(arr)) {
        agentNames = arr
          .filter((a: any) => a?.enabled !== false)
          .map((a: any) => {
            const slug: string = a?.kind ?? '';
            const display =
              slug === 'claude-code' ? 'Claude Code' :
              slug === 'opencode' ? 'OpenCode' :
              slug === 'hermes' ? 'Hermes' :
              slug === 'antigravity' ? 'Antigravity' :
              slug;
            return display;
          });
      }
    } catch {
      // ignore malformed JSON
    }
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
    };
  };

  const handleAdd = () => {
    setEditing({ ...blankForm, authType: 'key' });
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
      await invoke('test_machine_connection', { machineId: m.id });
      setTestState((s) => ({ ...s, [m.id]: 'ok' }));
    } catch (e: any) {
      setTestState((s) => ({ ...s, [m.id]: 'err' }));
      setTestErrors((s) => ({ ...s, [m.id]: String(e) }));
    }
  };

  const handleQuickDelete = async (m: Machine) => {
    if (!confirm(`Delete machine "${m.name}"? This removes its stored credentials.`)) return;
    try {
      await invoke('delete_machine', { id: m.id });
      try { await invoke('delete_machine_secret', { machineId: m.id }); } catch { /* ok */ }
      fetchMachines();
      onChange?.();
    } catch (e: any) {
      setError(String(e));
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
              SSH environments that Demeteo can run agents on. Each row is one machine with its own credentials.
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

        {loading ? (
          <div className="text-center py-12 text-slate-500 text-sm">
            <Loader className="w-5 h-5 animate-spin mx-auto mb-2 text-cyan-400" />
            Loading machines…
          </div>
        ) : machines.length === 0 ? (
          <div className="glass-panel p-12 text-center flex flex-col items-center justify-center">
            <Server className="w-10 h-10 text-slate-500 mb-3" />
            <h3 className="text-lg font-outfit font-semibold text-white mb-2">No machines configured</h3>
            <p className="text-sm text-slate-400 max-w-md mb-6">
              Add the local node or a remote SSH host where Demeteo can clone your repos and run coding agents.
            </p>
            <button
              onClick={handleAdd}
              className="px-5 py-2.5 text-sm font-bold rounded-lg bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all flex items-center gap-1.5"
            >
              <Plus className="w-4 h-4" />
              Add your first machine
            </button>
          </div>
        ) : (
          <div className="space-y-3">
            {machines.map((m) => {
              const conn = testState[m.id] ?? 'idle';
              const agents: string[] = (() => {
                try {
                  const arr = m.agents ? JSON.parse(m.agents) : [];
                  return Array.isArray(arr) ? arr.filter((a: any) => a?.enabled !== false).map((a: any) => a?.kind ?? '?') : [];
                } catch { return []; }
              })();
              const authLabel =
                m.auth_type === 'local' ? 'Local' :
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
                          {m.username ? <><span className="text-slate-200">@{m.username}</span>@</> : null}
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
                    </div>
                  </div>

                  <div className="flex items-center gap-1.5 shrink-0">
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