import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ShieldAlert, Check, Server, Code, GitBranch, X } from 'lucide-react';

interface ProviderSettingsProps {
    onConnected: (provider: { id: string; type: string; name: string; host: string; pat: string; username: string; avatarUrl: string }) => void;
    onClose: () => void;
    initialProvider?: { id: string; type: string; name: string; host: string };
}

export default function ProviderSettings({ onConnected, onClose, initialProvider }: ProviderSettingsProps) {
    const [providerType, setProviderType] = useState(initialProvider?.type || 'github');
    const [name, setName] = useState(initialProvider?.name || '');
    const [host, setHost] = useState(initialProvider?.host || '');
    const [pat, setPat] = useState('');
    const [status, setStatus] = useState<'idle' | 'validating' | 'success' | 'error'>('idle');
    const [errorMsg, setErrorMsg] = useState('');

    const handleConnect = async () => {
        if (!name.trim()) {
            setErrorMsg('Provider Name is required');
            setStatus('error');
            return;
        }
        if (!pat) {
            setErrorMsg('Personal Access Token is required');
            setStatus('error');
            return;
        }

        setStatus('validating');
        setErrorMsg('');

        try {
            const res: any = await invoke('connect_provider_instance', {
                providerType,
                host: host.trim(),
                pat: pat.trim()
            });

            setStatus('success');
            setTimeout(() => {
                onConnected({
                    id: res.id,
                    type: res.kind,
                    name: name.trim(),
                    host: res.host,
                    pat: 'hidden',
                    username: res.username,
                    avatarUrl: res.avatar_url
                });
            }, 1000);
        } catch (err: any) {
            setStatus('error');
            setErrorMsg(String(err));
        }
    };

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-[#08090c]/80 backdrop-blur-sm p-4">
            <div className="absolute w-[600px] h-[600px] bg-cyan-600/10 rounded-full blur-[120px] pointer-events-none"></div>
            
            <div className="glass-panel w-full max-w-xl p-8 relative z-10 flex flex-col shadow-2xl animate-fade-in border border-white/10">
                <button 
                    onClick={onClose}
                    className="absolute top-6 right-6 p-1.5 text-slate-400 hover:text-white rounded-md hover:bg-white/5 transition-colors"
                >
                    <X className="w-5 h-5" />
                </button>

                <div className="mb-6 border-b border-white/10 pb-4">
                    <h2 className="text-2xl font-outfit font-bold text-white mb-2">{initialProvider ? 'Edit Provider' : 'Connect a Provider'}</h2>
                    <p className="text-sm text-slate-400">Authenticate with GitHub or GitLab so the orchestrator can securely sync your worktrees.</p>
                </div>

                <div className="space-y-5">
                    <div>
                        <label className="block text-xs font-mono text-slate-400 mb-2 uppercase tracking-wider">Provider Type</label>
                        <div className="flex gap-4">
                            <button 
                                onClick={() => setProviderType('github')}
                                className={`flex-1 flex flex-col items-center gap-2 p-3 rounded-xl border transition-all ${providerType === 'github' ? 'bg-cyan-500/10 border-cyan-500/50 shadow-[0_0_15px_rgba(6,182,212,0.2)]' : 'bg-black/30 border-white/10 hover:border-white/20 hover:bg-white/5'}`}
                            >
                                <Code className={`w-6 h-6 ${providerType === 'github' ? 'text-cyan-400' : 'text-slate-500'}`} />
                                <span className={providerType === 'github' ? 'text-white font-medium text-sm' : 'text-slate-400 text-sm'}>GitHub</span>
                            </button>
                            <button 
                                onClick={() => setProviderType('gitlab')}
                                className={`flex-1 flex flex-col items-center gap-2 p-3 rounded-xl border transition-all ${providerType === 'gitlab' ? 'bg-violet-500/10 border-violet-500/50 shadow-[0_0_15px_rgba(139,92,246,0.2)]' : 'bg-black/30 border-white/10 hover:border-white/20 hover:bg-white/5'}`}
                            >
                                <GitBranch className={`w-6 h-6 ${providerType === 'gitlab' ? 'text-violet-400' : 'text-slate-500'}`} />
                                <span className={providerType === 'gitlab' ? 'text-white font-medium text-sm' : 'text-slate-400 text-sm'}>GitLab</span>
                            </button>
                        </div>
                    </div>

                    <div>
                        <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Provider Name / Alias</label>
                        <input 
                            type="text"
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                            placeholder="e.g. Personal GitHub, Corporate GitLab"
                            className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 transition-colors placeholder-slate-600"
                        />
                    </div>

                    <div>
                        <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Host URL (Optional)</label>
                        <div className="relative">
                            <Server className="absolute left-3 top-2.5 w-4 h-4 text-slate-500" />
                            <input 
                                type="text"
                                value={host}
                                onChange={(e) => setHost(e.target.value)}
                                placeholder={providerType === 'github' ? 'github.com' : 'gitlab.com'}
                                className="w-full bg-black/40 border border-white/10 rounded-lg py-2 pl-9 pr-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 transition-colors placeholder-slate-600"
                            />
                        </div>
                    </div>

                    <div>
                        <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Personal Access Token</label>
                        <input 
                            type="password"
                            value={pat}
                            onChange={(e) => setPat(e.target.value)}
                            placeholder="ghp_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"
                            className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 transition-colors font-mono placeholder-slate-600"
                        />
                        <p className="mt-1.5 text-[11px] text-slate-500">
                            Requires scopes: `repo` and `user`. The token is stored securely in your system keyring.
                        </p>
                    </div>

                    {status === 'error' && (
                        <div className="bg-ruby-500/10 border border-ruby-500/30 p-3 rounded-lg flex items-start gap-3">
                            <ShieldAlert className="w-5 h-5 text-ruby-400 shrink-0" />
                            <span className="text-sm text-ruby-200">{errorMsg}</span>
                        </div>
                    )}

                    {status === 'success' && (
                        <div className="bg-emerald-500/10 border border-emerald-500/30 p-3 rounded-lg flex items-center gap-3">
                            <div className="w-6 h-6 rounded-full bg-emerald-500 flex items-center justify-center shrink-0">
                                <Check className="w-4 h-4 text-black stroke-[3]" />
                            </div>
                            <span className="text-sm text-emerald-300 font-medium">Provider verified and connected successfully.</span>
                        </div>
                    )}
                </div>

                <div className="mt-6 flex justify-end gap-3">
                    <button 
                        onClick={onClose}
                        className="px-5 py-2 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-white/5 transition-all"
                    >
                        Cancel
                    </button>
                    <button 
                        onClick={handleConnect}
                        disabled={status === 'validating' || status === 'success'}
                        className={`px-6 py-2 rounded-lg font-medium shadow-lg transition-all text-sm flex items-center justify-center gap-2
                            ${status === 'success' ? 'bg-emerald-600 text-white shadow-emerald-500/20' : 
                              status === 'validating' ? 'bg-cyan-600/50 text-white cursor-wait' : 
                              'bg-cyan-600 hover:bg-cyan-500 text-white shadow-[0_0_15px_rgba(6,182,212,0.4)]'
                            }`}
                    >
                        {status === 'validating' ? 'Validating...' : status === 'success' ? 'Connected' : (initialProvider ? 'Update Account' : 'Connect Account')}
                    </button>
                </div>
            </div>
        </div>
    );
}
