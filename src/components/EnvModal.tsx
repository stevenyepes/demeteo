import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Server, X, Key, AlertCircle, Cpu, Wifi, WifiOff, Loader } from "lucide-react";


interface EnvFormState {
  id: string;
  name: string;
  connection: string;
  authType: string;
  keyPath: string;
  secret: string;
  agents: string[];
  useLoginShell: boolean;
  setupCommands: string;
}

interface EnvModalProps {
  isOpen: boolean;
  onClose: () => void;
  initialData: EnvFormState;
  /** Called after the machine + (optional) secret are persisted. */
  onSaved?: () => void;
  /** Called after the machine is deleted. */
  onDeleted?: () => void;
}

const blankForm: EnvFormState = {
  id: '',
  name: '',
  connection: '',
  authType: 'key',
  keyPath: '',
  secret: '',
  agents: [],
  useLoginShell: false,
  setupCommands: '',
};

const EnvModal: React.FC<EnvModalProps> = ({
  isOpen,
  onClose,
  initialData,
  onSaved,
  onDeleted,
}) => {
  const [form, setForm] = useState<EnvFormState>(initialData);
  const [connStatus, setConnStatus] = useState<"idle" | "testing" | "ok" | "err">("idle");
  const [connError, setConnError] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");

  useEffect(() => {
    setForm(initialData);
    setConnStatus("idle");
    setConnError("");
    setSaveError("");
  }, [initialData, isOpen]);

  if (!isOpen) return null;

  const toggleAgent = (agentName: string) => {
    const agents = form.agents || [];
    if (agents.includes(agentName)) {
      setForm({ ...form, agents: agents.filter((a) => a !== agentName) });
    } else {
      setForm({ ...form, agents: [...agents, agentName] });
    }
  };

  const handleBrowseKeyClick = async () => {
    try {
      const selected = await openDialog({
        multiple: false,
        directory: false,
        title: "Select private key file",
      });
      if (typeof selected === "string" && selected) {
        setForm((prev) => ({ ...prev, keyPath: selected }));
      }
    } catch (e) {
      console.warn("key browse failed:", e);
    }
  };

  /** Parse a "user@host:port" string into the parts the backend expects. */
  const parseConnection = (raw: string): { host: string; port: number; username: string } | null => {
    const trimmed = raw.trim();
    if (!trimmed) return null;
    // Accept "user@host", "user@host:port", "host", "host:port"
    let rest = trimmed;
    let username = "";
    const at = rest.indexOf("@");
    if (at >= 0) {
      username = rest.slice(0, at);
      rest = rest.slice(at + 1);
    }
    const colon = rest.lastIndexOf(":");
    let host = rest;
    let port = 22;
    if (colon > 0) {
      host = rest.slice(0, colon);
      const p = Number(rest.slice(colon + 1));
      if (Number.isFinite(p) && p > 0 && p < 65536) port = p;
    }
    if (!host) return null;
    return { username, host, port };
  };

  const handleTestConnection = async () => {
    const parsed = parseConnection(form.connection);
    if (!parsed) {
      setConnStatus("err");
      setConnError("Invalid connection string. Expected user@host or user@host:port.");
      return;
    }
    setConnStatus("testing");
    setConnError("");

    // Save first if dirty so test uses the latest secret.
    const machineId = await ensureSaved();
    if (!machineId) return;

    try {
      await invoke("test_machine_connection", { machineId });
      setConnStatus("ok");
    } catch (e: any) {
      setConnStatus("err");
      setConnError(String(e));
    }
  };

  /** Build the Machine payload the backend expects. */
  const buildMachine = (id: string) => {
    const parsed = parseConnection(form.connection) ?? { host: "", port: 22, username: "" };
    const agentsJson = JSON.stringify(
      (form.agents || []).map((name) => {
        const slug = name.toLowerCase().replace(/\s+/g, "-");
        return { kind: slug, enabled: true };
      }),
    );
    const setupJson = form.setupCommands.trim()
      ? JSON.stringify(form.setupCommands.split('\n').map(s => s.trim()).filter(Boolean))
      : null;
    return {
      id,
      name: form.name.trim(),
      host: parsed.host,
      port: parsed.port,
      username: parsed.username,
      auth_type: form.authType,
      key_path: form.authType === "key" && form.keyPath.trim() ? form.keyPath.trim() : null,
      agents: agentsJson,
      use_login_shell: form.useLoginShell,
      setup_commands: setupJson,
    };
  };

  /** Persist the machine + (if entered) the secret. Returns the id on success. */
  const ensureSaved = async (): Promise<string | null> => {
    setSaveError("");
    if (!form.name.trim()) {
      setSaveError("Environment name is required.");
      return null;
    }
    const parsed = parseConnection(form.connection);
    if (!parsed) {
      setSaveError("Invalid connection string.");
      return null;
    }
    if (form.authType === "key" && form.keyPath.trim().endsWith(".pub")) {
      setSaveError("That looks like a public key (.pub). Pick the private key.");
      return null;
    }
    setSaving(true);
    try {
      const id = form.id || `m-${Date.now()}`;
      const machine = buildMachine(id);
      if (form.id) {
        await invoke("update_machine", { machine });
      } else {
        await invoke("add_machine", { machine });
      }
      // Secret: only write when explicitly entered, so editing a
      // record without retyping the passphrase doesn't wipe the
      // stored one.
      if (form.secret.trim().length > 0) {
        await invoke("set_machine_secret", { machineId: id, secret: form.secret });
      }
      setForm((prev) => ({ ...prev, id }));
      onSaved?.();
      return id;
    } catch (e: any) {
      setSaveError(String(e));
      return null;
    } finally {
      setSaving(false);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const id = await ensureSaved();
    if (id) onClose();
  };

  const handleDelete = async () => {
    if (!form.id) return;
    if (!confirm(`Delete machine "${form.name}"? This removes its stored credentials.`)) return;
    try {
      await invoke("delete_machine", { id: form.id });
      try {
        await invoke("delete_machine_secret", { machineId: form.id });
      } catch {
        // Secret may not exist; ignore.
      }
      onDeleted?.();
      onClose();
    } catch (e: any) {
      setSaveError(String(e));
    }
  };

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4 select-none">
      <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-md shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
        <div className="px-6 py-4 border-b border-white/5 flex justify-between items-center bg-[#050508]">
          <h3 className="text-sm font-semibold text-white flex items-center">
            <Server size={16} className="mr-2 text-cyan-400" /> Configure Environment
          </h3>
          <button type="button" onClick={onClose} className="text-slate-500 hover:text-white transition-colors">
            <X size={16} />
          </button>
        </div>

        <form onSubmit={handleSubmit}>
          <div className="p-6 flex flex-col gap-4">
            <div>
              <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Environment Name</label>
              <input
                type="text"
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
                placeholder="e.g., prod-db-cluster"
                className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
              />
            </div>

            <div>
              <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Connection Details (User@Host:Port)</label>
              <input
                type="text"
                value={form.connection}
                onChange={(e) => setForm({ ...form, connection: e.target.value })}
                placeholder="e.g., ubuntu@10.0.5.12:22"
                className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
              />
            </div>

            <div>
              <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold flex items-center">
                <Key size={10} className="mr-1" /> Auth Method
              </label>
              <div className="flex flex-col gap-1">
                {(["key", "password", "agent"] as const).map((method) => {
                  const labels: Record<string, string> = {
                    key: "Private Key",
                    password: "Password",
                    agent: "SSH Agent",
                  };
                  return (
                    <button
                      key={method}
                      type="button"
                      onClick={() => setForm({ ...form, authType: method })}
                      className={`w-full text-left px-2.5 py-1.5 rounded-lg text-xs transition-all border ${
                        form.authType === method
                          ? "bg-cyan-500/10 border-cyan-500/40 text-cyan-400 font-medium"
                          : "bg-[#050508] border-white/5 text-slate-400 hover:border-white/15 hover:text-slate-300"
                      }`}
                    >
                      {labels[method]}
                    </button>
                  );
                })}
              </div>
            </div>

            {form.authType === "key" && (
              <div className="flex flex-col gap-3">
                <div>
                  <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Private Key Path</label>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={form.keyPath}
                      onChange={(e) => setForm({ ...form, keyPath: e.target.value })}
                      placeholder="~/.ssh/id_ed25519"
                      className={`flex-1 bg-[#050508] border rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50 ${
                        form.keyPath.endsWith(".pub") ? "border-amber-500/60" : "border-white/10"
                      }`}
                    />
                    <button
                      type="button"
                      onClick={handleBrowseKeyClick}
                      className="btn-secondary"
                      style={{ padding: "0 12px", fontSize: "0.8rem" }}
                    >
                      Browse
                    </button>
                  </div>
                  {form.keyPath.endsWith(".pub") && (
                    <p className="mt-1.5 text-[11px] text-amber-400 flex items-center gap-1">
                      ⚠ This looks like a public key. Use the private key file (without .pub).
                    </p>
                  )}
                </div>
                <div>
                  <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Key Passphrase (Optional)</label>
                  <input
                    type="password"
                    value={form.secret}
                    onChange={(e) => setForm({ ...form, secret: e.target.value })}
                    placeholder="Leave blank to keep the existing passphrase"
                    autoComplete="off"
                    className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
                  />
                  <p className="mt-1 text-[10px] text-slate-500">
                    Stored in the OS keyring. Only re-enter to change it.
                  </p>
                </div>
              </div>
            )}

            {form.authType === "password" && (
              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">SSH Password</label>
                <input
                  type="password"
                  value={form.secret}
                  onChange={(e) => setForm({ ...form, secret: e.target.value })}
                  placeholder="Leave blank to keep the existing password"
                  autoComplete="off"
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
                />
              </div>
            )}

            <div className="border-t border-white/5 pt-4">
              <div className="flex items-center text-amber-500 text-[11px] bg-amber-500/10 p-2.5 rounded-lg border border-amber-500/20">
                <AlertCircle size={14} className="mr-2 flex-shrink-0" />
                <span>Ensure public key authentication is configured on the target host.</span>
              </div>
            </div>

            <div>
              <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-2 font-semibold flex items-center">
                <Cpu size={10} className="mr-1" /> Enabled Agents
              </label>
              <div className="flex flex-wrap gap-2">
                {["Claude Code", "OpenCode", "Hermes"].map((agent) => (
                  <button
                    key={agent}
                    type="button"
                    onClick={() => toggleAgent(agent)}
                    className={`px-3 py-1.5 rounded-lg border text-xs font-mono transition-all ${
                      form.agents?.includes(agent)
                        ? "bg-cyan-500/10 border-cyan-500/50 text-cyan-400"
                        : "bg-[#050508] border-white/5 text-slate-500 hover:border-white/10"
                    }`}
                  >
                    {agent}
                  </button>
                ))}
              </div>
            </div>

            <div className="border-t border-white/5 pt-4 space-y-3">
              <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1 font-semibold flex items-center">
                <Server size={10} className="mr-1" /> Shell Environment
              </label>

              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={form.useLoginShell}
                  onChange={e => setForm({ ...form, useLoginShell: e.target.checked })}
                  className="w-4 h-4 rounded border-white/10 bg-[#050508] text-cyan-500 focus:ring-cyan-500/50"
                />
                <span className="text-xs text-slate-300">
                  Source .bashrc / .profile (login shell)
                </span>
                <span className="text-[10px] text-slate-500 ml-auto">
                  Env vars, mise, nvm, etc.
                </span>
              </label>

              <div>
                <label className="block text-[10px] text-slate-500 mb-1">
                  Setup commands (one per line, run after repo clone)
                </label>
                <textarea
                  value={form.setupCommands}
                  onChange={e => setForm({ ...form, setupCommands: e.target.value })}
                  placeholder={`mise trust /path/to/mise.toml`}
                  rows={3}
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 resize-none"
                />
              </div>
            </div>

            {saveError && (
              <div className="text-[11px] text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg p-2.5 break-all">
                {saveError}
              </div>
            )}
          </div>

          <div className="px-6 py-4 border-t border-white/5 bg-[#050508] flex flex-col gap-3">
            {/* Connection test result */}
            {connStatus === "ok" && (
              <div className="flex items-center text-emerald-400 text-xs bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2">
                <Wifi size={13} className="mr-2 flex-shrink-0" />
                SSH connection successful.
              </div>
            )}
            {connStatus === "err" && (
              <div className="flex items-start text-red-400 text-xs bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2">
                <WifiOff size={13} className="mr-2 flex-shrink-0 mt-0.5" />
                <div className="flex-1 break-all">{connError}</div>
                <button
                  type="button"
                  onClick={handleTestConnection}
                  className="ml-2 px-2 py-1 rounded border border-red-500/30 text-red-400 hover:bg-red-500/10 transition-colors text-[10px]"
                >
                  Retry
                </button>
              </div>
            )}

            <div className="flex justify-between gap-3">
              {form.id && (
                <button
                  type="button"
                  onClick={handleDelete}
                  className="px-4 py-2 rounded-lg text-xs font-medium text-red-400 border border-red-500/20 hover:bg-red-500/10 transition-colors"
                >
                  Delete Node
                </button>
              )}
              <div className="flex gap-2 ml-auto">
                <button
                  type="button"
                  onClick={handleTestConnection}
                  disabled={connStatus === "testing" || saving}
                  className="px-4 py-2 rounded-lg text-xs font-medium border border-white/10 text-slate-300 hover:border-cyan-500/40 hover:text-cyan-400 transition-colors flex items-center gap-1.5 disabled:opacity-50"
                >
                  {connStatus === "testing" ? (
                    <Loader size={12} className="animate-spin" />
                  ) : (
                    <Wifi size={12} />
                  )}
                  Test Connection
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={saving}
                  className="px-5 py-2 rounded-lg text-xs font-bold bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all disabled:opacity-50 flex items-center gap-1.5"
                >
                  {saving && <Loader size={12} className="animate-spin" />}
                  {form.id ? "Save Changes" : "Create Machine"}
                </button>
              </div>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
};

export default EnvModal;
export { blankForm };
export type { EnvFormState };