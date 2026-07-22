import React from 'react';
import { Rocket, Box, Database, Sparkles, Play } from 'lucide-react';

interface EmptyStateCardProps {
  onSeedSample: () => void;
  onConnectProviders: () => void;
  onSyncWorktrees: () => void;
  onDeployAgents: () => void;
  onCreateFromZero: () => void;
}

const EmptyStateCard: React.FC<EmptyStateCardProps> = ({
  onSeedSample,
  onConnectProviders,
  onSyncWorktrees,
  onDeployAgents,
  onCreateFromZero,
}) => {
  return (
    <div className="flex-1 flex items-center justify-center relative overflow-hidden">
      {/* Background radial gradients for that neon look. Static (no pulse):
          animating a 500px blur-[120px] element forces the WKWebView compositor
          to re-rasterize the blur every frame, which pinned the GPU process at
          idle. A static blur is painted once and cached. */}
      <div className="absolute top-1/4 left-1/4 w-[500px] h-[500px] bg-violet-600/20 rounded-full blur-[120px] pointer-events-none" />
      <div className="absolute bottom-1/4 right-1/4 w-[400px] h-[400px] bg-cyan-600/20 rounded-full blur-[100px] pointer-events-none" />

      <div className="glass-panel max-w-2xl w-full p-10 flex flex-col items-center text-center relative z-10 border-white/10 hover:border-white/20 transition-all duration-500">
        
        <div className="w-16 h-16 rounded-2xl bg-gradient-to-br from-violet-500/20 to-cyan-500/20 border border-white/10 flex items-center justify-center mb-6 shadow-[0_0_30px_rgba(139,92,246,0.2)]">
            <Sparkles className="w-8 h-8 text-cyan-400" />
        </div>

        <h1 className="font-outfit text-3xl font-bold text-white mb-4 tracking-tight">
          Welcome to the Demeteo Fleet Orchestrator
        </h1>
        
        <p className="text-slate-400 text-lg mb-8 max-w-lg leading-relaxed">
          Demeteo is a modern, premium control center for orchestrating multi-agent workflows. Connect your repositories, construct custom pipelines, and let specialized AI fleets handle the coding.
        </p>

        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-10 w-full">
          <button
            onClick={onConnectProviders}
            className="bg-black/30 border border-white/5 hover:border-violet-500/30 hover:bg-violet-950/10 rounded-xl p-4 flex flex-col items-center transition-all duration-300 hover:scale-105 cursor-pointer group text-center"
          >
            <Box className="w-6 h-6 text-violet-400 mb-3 group-hover:scale-110 transition-transform" />
            <span className="text-sm font-medium text-slate-300 group-hover:text-white transition-colors">Connect Providers</span>
          </button>
          <button
            onClick={onSyncWorktrees}
            className="bg-black/30 border border-white/5 hover:border-cyan-500/30 hover:bg-cyan-950/10 rounded-xl p-4 flex flex-col items-center transition-all duration-300 hover:scale-105 cursor-pointer group text-center"
          >
            <Database className="w-6 h-6 text-cyan-400 mb-3 group-hover:scale-110 transition-transform" />
            <span className="text-sm font-medium text-slate-300 group-hover:text-white transition-colors">Sync Worktrees</span>
          </button>
          <button
            onClick={onDeployAgents}
            className="bg-black/30 border border-white/5 hover:border-emerald-500/30 hover:bg-emerald-950/10 rounded-xl p-4 flex flex-col items-center transition-all duration-300 hover:scale-105 cursor-pointer group text-center"
          >
            <Rocket className="w-6 h-6 text-emerald-400 mb-3 group-hover:scale-110 transition-transform" />
            <span className="text-sm font-medium text-slate-300 group-hover:text-white transition-colors">Deploy Agents</span>
          </button>
          <button
            data-testid="empty-state-create-project"
            onClick={onCreateFromZero}
            className="relative bg-violet-950/30 border border-violet-500/40 hover:border-violet-400/70 hover:bg-violet-900/30 rounded-xl p-4 flex flex-col items-center transition-all duration-300 hover:scale-105 cursor-pointer group text-center overflow-hidden shadow-[0_0_25px_rgba(139,92,246,0.25)] hover:shadow-[0_0_35px_rgba(139,92,246,0.45)]"
          >
            <div className="absolute inset-0 bg-gradient-to-br from-violet-500/10 via-transparent to-cyan-500/10 pointer-events-none" />
            <Sparkles className="relative w-6 h-6 text-violet-300 mb-3 group-hover:scale-110 transition-transform" />
            <span className="relative text-sm font-outfit font-semibold text-violet-100 group-hover:text-white transition-colors">Create from Scratch</span>
            <span className="relative text-[10px] font-mono text-violet-300/70 mt-1 uppercase tracking-wider">Primary</span>
          </button>
        </div>

        <button 
          onClick={onSeedSample}
          className="group relative px-8 py-3 bg-violet-600 hover:bg-violet-500 text-white rounded-lg shadow-[0_0_20px_rgba(139,92,246,0.4)] transition-all font-medium flex items-center gap-3 overflow-hidden cursor-pointer"
        >
          <div className="absolute inset-0 bg-gradient-to-r from-transparent via-white/10 to-transparent -translate-x-full group-hover:translate-x-full transition-transform duration-700" />
          <Play className="w-5 h-5 fill-white/20" />
          Try a sample project
        </button>
        <p className="mt-4 text-xs text-slate-500 font-mono">
          No API keys required to explore the demo workspace.
        </p>

      </div>
    </div>
  );
};

export default EmptyStateCard;
