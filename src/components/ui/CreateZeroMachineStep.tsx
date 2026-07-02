import { HardDrive, Key, Server } from 'lucide-react';
import type { Machine } from '../../types';

export interface CreateZeroMachineStepProps {
  machineKind: 'local' | 'remote';
  machineId: string;
  machines: ReadonlyArray<Machine>;
  keyPassphrase: string;
  onMachineKindChange: (kind: 'local' | 'remote') => void;
  onMachineIdChange: (id: string) => void;
  onKeyPassphraseChange: (value: string) => void;
}

/**
 * Step 3 — local vs remote. Renders the kind toggle, the SSH machine
 * dropdown (only when remote is chosen), and the key-passphrase
 * sub-field (only when the chosen machine uses key auth).
 */
export function CreateZeroMachineStep(props: CreateZeroMachineStepProps) {
  const {
    machineKind, machineId, machines, keyPassphrase,
    onMachineKindChange, onMachineIdChange, onKeyPassphraseChange,
  } = props;
  const selectedMachine = machines.find((m) => m.id === machineId) ?? null;
  const showPassphrase = machineKind === 'remote' && selectedMachine?.auth_type === 'key';

  return (
    <div className="space-y-4">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block">
        Where should agents run?
      </label>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <button
          type="button"
          onClick={() => { onMachineKindChange('local'); onMachineIdChange(''); }}
          className={`flex items-center gap-3 p-3 rounded-lg border text-left text-sm transition-all ${
            machineKind === 'local'
              ? 'bg-violet-500/10 border-violet-500/50 text-violet-100'
              : 'bg-black/40 border-white/10 text-slate-300 hover:border-white/20'
          }`}
        >
          <HardDrive className="w-4 h-4" />
          <div>
            <div className="font-medium">Local compute</div>
            <div className="text-[10px] font-mono text-slate-500">Run on this workstation</div>
          </div>
        </button>
        <button
          type="button"
          onClick={() => onMachineKindChange('remote')}
          className={`flex items-center gap-3 p-3 rounded-lg border text-left text-sm transition-all ${
            machineKind === 'remote'
              ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-100'
              : 'bg-black/40 border-white/10 text-slate-300 hover:border-white/20'
          }`}
        >
          <Server className="w-4 h-4" />
          <div>
            <div className="font-medium">Remote SSH</div>
            <div className="text-[10px] font-mono text-slate-500">Run on a configured machine</div>
          </div>
        </button>
      </div>

      {machineKind === 'remote' && (
        <div className="space-y-2">
          <select
            value={machineId}
            onChange={(e) => onMachineIdChange(e.target.value)}
            className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white font-mono focus:outline-none focus:border-cyan-500/50"
          >
            <option value="">
              {machines.length === 0 ? 'No machines configured — add one in Settings → Machines' : 'Select a machine…'}
            </option>
            {machines.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name} ({m.username}@{m.host}:{m.port} — {m.auth_type})
              </option>
            ))}
          </select>
          {showPassphrase && (
            <div>
              <label className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5 flex items-center gap-1.5">
                <Key className="w-3 h-3" /> Private key passphrase
              </label>
              <input
                type="password"
                value={keyPassphrase}
                onChange={(e) => onKeyPassphraseChange(e.target.value)}
                placeholder="Leave blank if the key has no passphrase"
                autoComplete="off"
                className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
              />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
