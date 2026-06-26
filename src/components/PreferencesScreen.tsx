import React, { useState, useEffect } from 'react';
import { Settings, Server, Globe, Cpu, Info, Activity, FolderOpen, Check, RotateCw, Brain } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import MachinesView from './MachinesView';
import MemoryAgentSettings from './MemoryAgentSettings';

type PrefTab = 'machines' | 'providers' | 'defaults' | 'memory' | 'about';

interface PreferencesScreenProps {
  onNavigate: (view: string) => void;
}

const PreferencesScreen: React.FC<PreferencesScreenProps> = ({ onNavigate }) => {
  const [activeTab, setActiveTab] = useState<PrefTab>('machines');

  // Workspace directory state
  const [effectiveWorkspaceDir, setEffectiveWorkspaceDir] = useState('');
  const [workspaceDirInput, setWorkspaceDirInput] = useState('');
  const [workspaceDirSaving, setWorkspaceDirSaving] = useState(false);
  const [workspaceDirSaved, setWorkspaceDirSaved] = useState(false);

  useEffect(() => {
    if (activeTab !== 'defaults') return;
    (async () => {
      const [effective, override] = await Promise.all([
        invoke<string>('get_workspace_dir'),
        invoke<string | null>('get_workspace_dir_setting'),
      ]);
      setEffectiveWorkspaceDir(effective);
      setWorkspaceDirInput(override ?? '');
    })();
  }, [activeTab]);

  const handleBrowseWorkspaceDir = async () => {
    const selected = await openDialog({ directory: true, multiple: false, title: 'Choose workspace directory' });
    if (selected && typeof selected === 'string') {
      setWorkspaceDirInput(selected);
    }
  };

  const handleSaveWorkspaceDir = async () => {
    setWorkspaceDirSaving(true);
    try {
      await invoke('set_workspace_dir_setting', { path: workspaceDirInput || null });
      const effective = await invoke<string>('get_workspace_dir');
      setEffectiveWorkspaceDir(effective);
      setWorkspaceDirSaved(true);
      setTimeout(() => setWorkspaceDirSaved(false), 2500);
    } finally {
      setWorkspaceDirSaving(false);
    }
  };

  const tabs: { key: PrefTab; label: string; icon: React.ReactNode }[] = [
    { key: 'machines', label: 'Machines', icon: <Server className="w-4 h-4" /> },
    { key: 'providers', label: 'Providers', icon: <Globe className="w-4 h-4" /> },
    { key: 'defaults', label: 'Defaults', icon: <Cpu className="w-4 h-4" /> },
    { key: 'memory', label: 'Memory', icon: <Brain className="w-4 h-4" /> },
    { key: 'about', label: 'About', icon: <Info className="w-4 h-4" /> },
  ];

  return (
    <div className="flex-1 overflow-y-auto relative">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-violet-600/5 rounded-full blur-[120px] pointer-events-none" />

      <div className="max-w-5xl mx-auto relative z-10 p-8 pt-6">
        {/* Header */}
        <div className="flex items-center justify-between mb-6 border-b border-white/5 pb-4">
          <div className="flex items-center gap-3">
            <Settings className="w-6 h-6 text-cyan-400" />
            <div>
              <h1 className="text-2xl font-outfit font-bold text-white">Preferences</h1>
              <p className="text-sm text-slate-400">Global settings for Demeteo orchestrator</p>
            </div>
          </div>
          <button
            onClick={() => onNavigate('home')}
            className="px-4 py-2 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 transition-colors"
          >
            Back to Projects
          </button>
        </div>

        {/* Tab bar */}
        <div className="flex gap-1 mb-6 border-b border-white/5 pb-px">
          {tabs.map(tab => (
            <button
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              className={`flex items-center gap-2 px-4 py-2.5 text-xs font-medium border-b-2 transition-all ${
                activeTab === tab.key
                  ? 'border-cyan-500 text-cyan-300'
                  : 'border-transparent text-slate-400 hover:text-slate-200'
              }`}
            >
              {tab.icon}
              {tab.label}
            </button>
          ))}
        </div>

        {/* Tab content */}
        {activeTab === 'machines' && (
          <MachinesView onChange={() => {}} />
        )}
        {activeTab === 'providers' && (
          <div className="glass-panel p-6 text-center">
            <Globe className="w-8 h-8 text-slate-500 mx-auto mb-3" />
            <h3 className="text-sm font-outfit font-semibold text-white mb-1">Provider Management</h3>
            <p className="text-xs text-slate-400 mb-4">Manage Git hosting provider connections.</p>
            <button
              onClick={() => onNavigate('providers')}
              className="px-4 py-2 text-xs font-medium bg-cyan-600 hover:bg-cyan-500 text-white rounded-lg transition-all"
            >
              Open Providers Page
            </button>
          </div>
        )}
        {activeTab === 'defaults' && (
          <div className="space-y-4">
            {/* Workspace Storage */}
            <div className="glass-panel p-6">
              <h3 className="text-sm font-outfit font-semibold text-white mb-1 flex items-center gap-2">
                <FolderOpen className="w-4 h-4 text-cyan-400" />
                Workspace Storage
              </h3>
              <p className="text-xs text-slate-400 mb-4">
                Where Demeteo clones project repositories. Defaults to the app data directory.
                Changes take effect after restarting the app; existing projects will need re-bootstrapping
                if you move the directory.
              </p>

              <div className="space-y-3">
                <div>
                  <label className="block text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1.5">
                    Active directory
                  </label>
                  <p className="font-mono text-xs text-slate-300 bg-black/40 border border-white/5 rounded-lg px-3 py-2 break-all">
                    {effectiveWorkspaceDir || '…'}
                  </p>
                </div>

                <div>
                  <label className="block text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1.5">
                    Custom path override <span className="normal-case text-slate-600">(leave blank to use default)</span>
                  </label>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={workspaceDirInput}
                      onChange={e => setWorkspaceDirInput(e.target.value)}
                      placeholder={effectiveWorkspaceDir}
                      className="flex-1 bg-black/40 border border-white/10 rounded-lg px-3 py-2 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                    />
                    <button
                      onClick={handleBrowseWorkspaceDir}
                      className="px-3 py-2 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 transition-all flex items-center gap-1.5 shrink-0"
                    >
                      <FolderOpen className="w-3.5 h-3.5" />
                      Browse
                    </button>
                    <button
                      onClick={handleSaveWorkspaceDir}
                      disabled={workspaceDirSaving}
                      className="px-4 py-2 text-xs font-medium rounded-lg bg-cyan-600 hover:bg-cyan-500 disabled:opacity-50 text-white transition-all flex items-center gap-1.5 shrink-0"
                    >
                      {workspaceDirSaving ? (
                        <RotateCw className="w-3 h-3 animate-spin" />
                      ) : workspaceDirSaved ? (
                        <Check className="w-3 h-3" />
                      ) : null}
                      {workspaceDirSaved ? 'Saved' : 'Save'}
                    </button>
                  </div>
                  {workspaceDirSaved && (
                    <p className="mt-1.5 text-[10px] text-amber-400 font-mono">
                      Restart the app to apply the new workspace directory.
                    </p>
                  )}
                </div>
              </div>
            </div>

            {/* Default Agent & Model */}
            <div className="glass-panel p-6">
              <h3 className="text-sm font-outfit font-semibold text-white mb-3 flex items-center gap-2">
                <Cpu className="w-4 h-4 text-violet-400" />
                Default Agent & Model
              </h3>
              <p className="text-xs text-slate-400 mb-4">
                Default agent and model settings are configured per-project in the Workspace Settings screen.
                Global defaults will be available in a future release.
              </p>
              <div className="bg-black/40 border border-white/5 rounded-lg p-3 text-xs text-slate-400">
                <p>To configure defaults for a specific project:</p>
                <ol className="list-decimal ml-4 mt-2 space-y-1 text-slate-500">
                  <li>Open the project from the sidebar</li>
                  <li>Click the <strong className="text-slate-300">Settings</strong> icon</li>
                  <li>Go to <strong className="text-slate-300">Agent Strategy &amp; Policies</strong></li>
                  <li>Set the default agent kind and model</li>
                </ol>
              </div>
            </div>
          </div>
        )}
        {activeTab === 'memory' && <MemoryAgentSettings />}
        {activeTab === 'about' && (
          <div className="glass-panel p-6 space-y-4">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-lg bg-gradient-to-br from-violet-500/20 to-cyan-500/20 border border-white/10 flex items-center justify-center">
                <Activity className="w-5 h-5 text-cyan-400" />
              </div>
              <div>
                <h3 className="text-base font-outfit font-bold text-white">Demeteo</h3>
                <p className="text-xs text-slate-400">Multi-Agent Orchestrator v0.1.0</p>
              </div>
            </div>
            <div className="border-t border-white/5 pt-4 text-xs text-slate-400 space-y-2">
              <p>A modern, premium desktop control center for orchestrating local and remote AI agents.</p>
              <p>Built with Tauri v2, React, Rust, and TypeScript.</p>
              <p className="text-slate-500 pt-2">
                Demeteo plays with the Spanish language (monitoreo) and classical Greek mythology
                — Demeter (goddess of agriculture) and Prometheus (Titan of foresight).
              </p>
            </div>
            <div className="border-t border-white/5 pt-4 text-[10px] text-slate-600 font-mono">
              <p>Data directory: ~/.demeteo/</p>
              <p>Logs: ~/.demeteo/logs/</p>
              <p>Artifacts: ~/.demeteo/artifacts/</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default PreferencesScreen;
