import { HardDrive, Key, Server } from 'lucide-react';
import type { Machine } from '../../types';
import type { CreateProjectStepPayload } from '../../types';

export interface MachineStepProps {
  machines: ReadonlyArray<Machine>;
  /** `local` or `remote` — the kind the user has toggled to. */
  kind: 'local' | 'remote';
  /** Selected remote machine id (empty string when none yet picked). */
  machineId: string;
  /** Passphrase for the SSH key (only relevant when the selected
   *  remote machine uses key auth). Cleared from in-memory state
   *  after the wizard commits — see `CreateProjectWizard`. */
  keyPassphrase: string;
  /** Emits the typed `{ step: 'machine', kind, machineId }` payload
   *  that matches the Rust `CreateProjectStepPayload::Machine`
   *  variant. The passphrase is **not** part of this payload — it
   *  travels through {@link onPassphraseChange} instead and is
   *  written to the OS keyring via `set_machine_secret` before
   *  bootstrap runs. */
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'machine' }>) => void;
  /** Auxiliary non-payload setter for the passphrase. Strictly the
   *  Rust payload does not carry a passphrase, so the orchestrator
   *  holds it in React state and writes it to the keyring itself. */
  onPassphraseChange: (value: string) => void;
}

/**
 * Step 4 — Machine. Big local/remote toggle plus a machine
 * dropdown (only when remote is selected) and a key-passphrase
 * field (only when the selected machine uses key auth). Emits the
 * matching `{ step: 'machine', kind, machineId }` payload upward
 * via {@link MachineStepProps.onSubmit}.
 */
export function MachineStep({
  machines, kind, machineId, keyPassphrase, onSubmit, onPassphraseChange,
}: MachineStepProps) {
  const selectedMachine = machines.find((m) => m.id === machineId) ?? null;
  const showPassphrase = kind === 'remote' && selectedMachine?.auth_type === 'key';

  const emit = (next: { kind?: 'local' | 'remote'; machineId?: string | null }) => {
    onSubmit({
      step: 'machine',
      kind: next.kind ?? kind,
      machineId: next.machineId !== undefined ? next.machineId : (machineId || null),
    });
  };

  return (
    <div className="space-y-4" data-testid="wizard-step-machine">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block">
        Where should agents run?
      </label>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <button
          type="button"
          onClick={() => emit({ kind: 'local', machineId: null })}
          data-testid="wizard-machine-local"
          className={`flex items-center gap-3 p-3 rounded-lg border text-left text-sm transition-all ${
            kind === 'local'
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
          onClick={() => emit({ kind: 'remote' })}
          data-testid="wizard-machine-remote"
          className={`flex items-center gap-3 p-3 rounded-lg border text-left text-sm transition-all ${
            kind === 'remote'
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

      {kind === 'remote' && (
        <div className="space-y-2">
          <select
            value={machineId}
            onChange={(e) => emit({ machineId: e.target.value || null })}
            data-testid="wizard-machine-select"
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
              <label
                htmlFor="wizard-machine-passphrase"
                className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5 flex items-center gap-1.5"
              >
                <Key className="w-3 h-3" /> Private key passphrase
              </label>
              <input
                id="wizard-machine-passphrase"
                type="password"
                value={keyPassphrase}
                onChange={(e) => onPassphraseChange(e.target.value)}
                placeholder="Leave blank if the key has no passphrase"
                autoComplete="off"
                className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
              />
              <p className="mt-1 text-[10px] text-slate-500 font-mono">
                Written to the OS keyring via <code>set_machine_secret</code> before bootstrap.
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}