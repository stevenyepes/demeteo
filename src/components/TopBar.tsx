import { Search, Sliders, Globe, Settings } from 'lucide-react';
import { NotificationBell } from './NotificationBell';
import { useNavigation, useUIState } from '../context';
import type { Provider } from '../types';

interface TopBarProps {
  connectedProvider: Provider | null;
}

function TopBar({ connectedProvider }: TopBarProps) {
  const { navigate } = useNavigation();
  const { uiDispatch } = useUIState();

  return (
    <header className="h-14 border-b border-white/5 bg-[#0d0f14]/80 backdrop-blur-md flex items-center justify-between px-6 z-20 relative shrink-0">
      <div className="flex items-center gap-4">
        <img src="/icon.png" alt="Demeteo" className="w-8 h-8 rounded-lg" />
        <h1 className="font-outfit font-bold tracking-wide text-lg text-white">demeteo</h1>
      </div>

      <div className="flex items-center gap-4">
        <div
          className="flex items-center px-3 py-1.5 glass-panel rounded-md text-sm text-slate-400 w-64 group hover:border-white/20 transition-colors cursor-pointer"
          onClick={() => uiDispatch({ type: 'SET_COMMAND_PALETTE', open: true })}
        >
          <Search className="w-4 h-4 mr-2 opacity-50" />
          <span>Search workspace...</span>
          <span className="ml-auto text-[10px] font-mono border border-white/10 px-1.5 py-0.5 rounded opacity-50">⌘K</span>
        </div>
        <div className="w-px h-5 bg-white/10" />
        <button onClick={() => navigate({ kind: 'workflows' })} className="text-slate-400 hover:text-white transition-all hover:bg-white/5 p-1.5 rounded flex items-center gap-1 text-xs" title="Templates Hub">
          <Sliders className="w-4 h-4 text-violet-400" />
          <span className="hidden md:inline font-mono">Workflows</span>
        </button>
        <button onClick={() => navigate({ kind: 'providers' })} className="text-slate-400 hover:text-white transition-all hover:bg-white/5 p-1.5 rounded flex items-center gap-1 text-xs" title="Source Providers">
          <Globe className="w-4 h-4 text-cyan-400" />
          <span className="hidden md:inline font-mono">Providers</span>
        </button>
        <NotificationBell />
        <button onClick={() => navigate({ kind: 'settings' })} className="text-slate-400 hover:text-white transition-colors hover:bg-white/5 p-1.5 rounded">
          <Settings className="w-5 h-5" />
        </button>
        {connectedProvider?.avatarUrl ? (
          <img src={connectedProvider.avatarUrl} alt={connectedProvider.username} className="w-8 h-8 rounded-full border-2 border-cyan-500/50 ml-2 object-cover" />
        ) : (
          <div className="w-8 h-8 rounded-full bg-gradient-to-tr from-violet-600 to-cyan-600 border-2 border-white/10 ml-2" />
        )}
      </div>
    </header>
  );
};

export default TopBar;
