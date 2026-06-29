import { useState } from 'react';
import { Globe, Plus, Edit2, Trash2, AlertTriangle, X } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import type { Provider } from '../types';
import { useProject, useUIState } from '../context';
import { useErrorBus } from '../lib/errorBus';
import ProviderSettings from './ProviderSettings';

export default function ProvidersPage() {
  const { state: { providers, projects, reposByProject }, dispatch: projDispatch } = useProject();
  const { ui, uiDispatch } = useUIState();
  const { isConnectModalOpen, editingProvider } = ui;
  const { reportError } = useErrorBus();
  const [pendingDelete, setPendingDelete] = useState<{ provId: string; affectedProjects: string[] } | null>(null);

  const handleProviderConnected = (newProv: Provider) => {
    projDispatch({
      type: 'SET_PROVIDERS',
      providers: providers.some(p => p.id === newProv.id)
        ? providers.map(p => p.id === newProv.id ? newProv : p)
        : [...providers, newProv],
    });
    uiDispatch({ type: 'SET_CONNECT_MODAL', open: false, editing: null });
  };

  const handleDeleteClick = (provId: string) => {
    const affected = projects.filter(proj =>
      (reposByProject[proj.id] ?? []).some(r => r.provider_id === provId)
    ).map(proj => proj.name);
    if (affected.length > 0) {
      setPendingDelete({ provId, affectedProjects: affected });
    } else {
      confirmDelete(provId);
    }
  };

  const confirmDelete = async (provId: string) => {
    setPendingDelete(null);
    try {
      await invoke('delete_provider_instance', { providerId: provId });
      projDispatch({ type: 'SET_PROVIDERS', providers: providers.filter(p => p.id !== provId) });
    } catch (err) { reportError(err, { kind: 'provider' }); }
  };

  return (
    <div className="flex-1 overflow-y-auto p-8 relative flex flex-col justify-start max-w-4xl mx-auto w-full">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-cyan-600/5 rounded-full blur-[120px] pointer-events-none" />

      <div className="flex justify-between items-center mb-8 border-b border-white/5 pb-4 z-10">
        <div>
          <h1 className="text-2xl font-outfit font-bold text-white mb-2">Source Providers</h1>
          <p className="text-sm text-slate-400">Manage Git hosting endpoints for cloning repositories and creating merge requests.</p>
        </div>
        <button onClick={() => uiDispatch({ type: 'SET_CONNECT_MODAL', open: true })} className="bg-cyan-600 hover:bg-cyan-500 text-white font-medium text-sm px-4 py-2 rounded-lg transition-all shadow-[0_0_15px_rgba(6,182,212,0.3)] flex items-center gap-2">
          <Plus className="w-4 h-4" /> Connect Provider
        </button>
      </div>

      <div className="space-y-4 z-10">
        {providers.length === 0 ? (
          <div className="glass-panel p-12 text-center flex flex-col items-center justify-center">
            <Globe className="w-12 h-12 text-slate-500 mb-4 animate-pulse" />
            <h3 className="text-lg font-outfit font-semibold text-white mb-2">No Providers Mapped</h3>
            <p className="text-sm text-slate-400 max-w-md mb-6">Connect your GitHub or GitLab workspace to enable repository cloning, branch management, and automatic pull requests.</p>
            <button onClick={() => uiDispatch({ type: 'SET_CONNECT_MODAL', open: true })} className="bg-cyan-600 hover:bg-cyan-500 text-white font-medium text-sm px-5 py-2.5 rounded-lg transition-all shadow-[0_0_15px_rgba(6,182,212,0.3)]">
              Connect Your First Account
            </button>
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {providers.map(prov => (
              <div key={prov.id} className="glass-panel p-5 flex items-start justify-between border-l-2 border-l-cyan-500 hover:border-l-cyan-400 transition-all">
                <div className="flex gap-4">
                  {prov.avatarUrl ? (
                    <img src={prov.avatarUrl} alt={prov.username} className="w-12 h-12 rounded-full object-cover border border-white/10" />
                  ) : (
                    <div className="w-12 h-12 rounded-full bg-gradient-to-tr from-violet-600 to-cyan-600 flex items-center justify-center border border-white/10 text-white font-bold text-lg">
                      {prov.name.charAt(0).toUpperCase()}
                    </div>
                  )}
                  <div>
                    <h4 className="text-base font-semibold text-white font-outfit">{prov.name}</h4>
                    <div className="text-xs text-slate-400 mt-1 space-y-0.5 font-mono">
                      <p>User: <span className="text-slate-200">@{prov.username}</span></p>
                      <p>Host: <span className="text-slate-200">{prov.host}</span></p>
                      <p>Type: <span className="text-slate-200 capitalize">{prov.type}</span></p>
                    </div>
                  </div>
                </div>
                <div className="flex gap-2">
                  <button onClick={() => uiDispatch({ type: 'SET_CONNECT_MODAL', open: true, editing: prov })} className="text-slate-500 hover:text-cyan-400 p-2 rounded-lg hover:bg-white/5 transition-all" title="Edit Provider">
                    <Edit2 className="w-4 h-4" />
                  </button>
                  <button onClick={() => handleDeleteClick(prov.id)} className="text-slate-500 hover:text-ruby-400 p-2 rounded-lg hover:bg-white/5 transition-all" title="Disconnect Provider">
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {isConnectModalOpen && (
        <ProviderSettings
          initialProvider={editingProvider || undefined}
          onConnected={handleProviderConnected}
          onClose={() => uiDispatch({ type: 'SET_CONNECT_MODAL', open: false, editing: null })}
        />
      )}

      {pendingDelete && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="glass-panel p-6 max-w-md w-full mx-4 border border-amber-500/30">
            <div className="flex items-start gap-3 mb-4">
              <AlertTriangle className="w-5 h-5 text-amber-400 mt-0.5 shrink-0" />
              <div>
                <h3 className="text-base font-semibold text-white font-outfit mb-1">Provider in use</h3>
                <p className="text-sm text-slate-400">
                  The following {pendingDelete.affectedProjects.length === 1 ? 'project uses' : 'projects use'} this provider.
                  Removing it will break cloning and MR publishing until you reconnect.
                </p>
              </div>
              <button onClick={() => setPendingDelete(null)} className="ml-auto text-slate-500 hover:text-slate-300">
                <X className="w-4 h-4" />
              </button>
            </div>
            <ul className="mb-5 space-y-1">
              {pendingDelete.affectedProjects.map(name => (
                <li key={name} className="text-sm text-slate-200 font-mono bg-white/5 rounded px-3 py-1">{name}</li>
              ))}
            </ul>
            <div className="flex gap-3 justify-end">
              <button onClick={() => setPendingDelete(null)} className="px-4 py-2 text-sm text-slate-300 hover:text-white rounded-lg hover:bg-white/5 transition-all">
                Cancel
              </button>
              <button onClick={() => confirmDelete(pendingDelete.provId)} className="px-4 py-2 text-sm bg-red-600/80 hover:bg-red-500 text-white rounded-lg transition-all">
                Disconnect anyway
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
