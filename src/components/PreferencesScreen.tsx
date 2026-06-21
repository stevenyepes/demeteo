import React, { useState } from 'react';
import { Settings, Server, Globe, Cpu, Info, Activity } from 'lucide-react';
import MachinesView from './MachinesView';

type PrefTab = 'machines' | 'providers' | 'defaults' | 'about';

interface PreferencesScreenProps {
  onNavigate: (view: string) => void;
}

const PreferencesScreen: React.FC<PreferencesScreenProps> = ({ onNavigate }) => {
  const [activeTab, setActiveTab] = useState<PrefTab>('machines');

  const tabs: { key: PrefTab; label: string; icon: React.ReactNode }[] = [
    { key: 'machines', label: 'Machines', icon: <Server className="w-4 h-4" /> },
    { key: 'providers', label: 'Providers', icon: <Globe className="w-4 h-4" /> },
    { key: 'defaults', label: 'Defaults', icon: <Cpu className="w-4 h-4" /> },
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
        )}
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
