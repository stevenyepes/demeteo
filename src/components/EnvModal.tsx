import React, { useState, useEffect } from "react";
import { Server, X, Key, AlertCircle, Cpu, Wifi, WifiOff, Loader } from "lucide-react";


interface EnvFormState {
  id: string;
  name: string;
  connection: string;
  authType: string;
  keyPath: string;
  secret: string;
  agents: string[];
}

interface EnvModalProps {
  isOpen: boolean;
  onClose: () => void;
  initialData: EnvFormState;
  onSave: (data: EnvFormState) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onBrowseKey: () => Promise<string | null>;
  onTestConnection: (form: EnvFormState) => Promise<string>;
}

const EnvModal: React.FC<EnvModalProps> = ({
  isOpen,
  onClose,
  initialData,
  onSave,
  onDelete,
  onBrowseKey,
  onTestConnection,
}) => {
  const [form, setForm] = useState<EnvFormState>(initialData);
  const [connStatus, setConnStatus] = useState<"idle" | "testing" | "ok" | "err">("idle");
  const [connError, setConnError] = useState("");

  useEffect(() => {
    setForm(initialData);
    setConnStatus("idle");
    setConnError("");
  }, [initialData]);

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
    const selected = await onBrowseKey();
    if (selected) {
      setForm((prev) => ({ ...prev, keyPath: selected }));
    }
  };

  const handleTestConnection = async () => {
    setConnStatus("testing");
    setConnError("");
    const result = await onTestConnection(form as any);
    if (result === "ok") {
      setConnStatus("ok");
    } else {
      setConnStatus("err");
      setConnError(result);
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSave(form);
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
                className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-2 font-semibold">Type</label>
                <div className="flex flex-col gap-2">
                  <label className="flex items-center text-xs text-slate-300 cursor-pointer">
                    <input
                      type="radio"
                      name="envType"
                      value="key"
                      checked={form.authType !== "local"}
                      onChange={() => setForm({ ...form, authType: "key" })}
                      className="mr-2 accent-cyan-500"
                    />
                    Remote SSH Server
                  </label>
                  <label className="flex items-center text-xs text-slate-300 cursor-pointer">
                    <input
                      type="radio"
                      name="envType"
                      value="local"
                      checked={form.authType === "local"}
                      onChange={() => setForm({ ...form, authType: "local" })}
                      className="mr-2 accent-cyan-500"
                    />
                    Local Node
                  </label>
                </div>
              </div>

              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold flex items-center">
                  <Key size={10} className="mr-1" /> Auth Method
                </label>
                {form.authType !== "local" ? (
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
                ) : (
                  <div className="text-[11px] text-slate-400 bg-[#050508] border border-white/5 rounded-lg p-2 font-mono">
                    Native API (Local)
                  </div>
                )}
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
                  <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Key Passphrase / Password (Optional)</label>
                  <input
                    type="password"
                    value={form.secret}
                    onChange={(e) => setForm({ ...form, secret: e.target.value })}
                    placeholder="Leave blank to keep current secret"
                    className="w-full bg-[#050508] border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
                  />
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
                  placeholder="SSH password details"
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
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete(form.id);
                  }}
                  className="px-4 py-2 rounded-lg text-xs font-medium text-red-400 border border-red-500/20 hover:bg-red-500/10 transition-colors"
                >
                  Delete Node
                </button>
              )}
              <div className="flex gap-2 ml-auto">
                {form.authType !== "local" && (
                  <button
                    type="button"
                    onClick={handleTestConnection}
                    disabled={connStatus === "testing"}
                    className="px-4 py-2 rounded-lg text-xs font-medium border border-white/10 text-slate-300 hover:border-cyan-500/40 hover:text-cyan-400 transition-colors flex items-center gap-1.5 disabled:opacity-50"
                  >
                    {connStatus === "testing" ? (
                      <Loader size={12} className="animate-spin" />
                    ) : (
                      <Wifi size={12} />
                    )}
                    Test Connection
                  </button>
                )}
                <button
                  type="button"
                  onClick={onClose}
                  className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="px-5 py-2 rounded-lg text-xs font-bold bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all"
                >
                  Save Environment
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
