import { useState, useMemo } from 'react';
import { Plus, Search, Box, GitBranch, PanelLeftOpen, PanelLeftClose } from 'lucide-react';
import { StatusBadge } from './ui/StatusBadge';
import { useNavigation, useProject, useUIState } from '../context';

function fuzzyMatch(text: string, query: string): boolean {
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  let qi = 0;
  for (let i = 0; i < lower.length && qi < q.length; i++) {
    if (lower[i] === q[qi]) qi++;
  }
  return qi === q.length;
}

const statusLabel: Record<string, string> = {
  idle: 'Ready',
  active: 'Active',
  running: 'Running',
  bootstrapping: 'Bootstrapping',
  gated: 'Gate Required',
  error: 'Error',
  failed: 'Failed',
};

function ProjectRail() {
  const { navigate } = useNavigation();
  const { state: { projects, currentProjectId }, dispatch } = useProject();
  const { ui: { sidebarCollapsed }, uiDispatch } = useUIState();
  const collapsed = sidebarCollapsed;
  const currentProject = currentProjectId;
  const setCurrentProject = (id: string) => { dispatch({ type: 'SET_CURRENT', id }); navigate({ kind: 'home' }); };
  const onToggleCollapse = () => uiDispatch({ type: 'TOGGLE_SIDEBAR' });
  const setView = (v: string) => navigate({ kind: v as any });

  const [searchQuery, setSearchQuery] = useState('');

  const filtered = useMemo(() => {
    if (!searchQuery.trim()) return projects;
    return projects.filter(p => fuzzyMatch(p.name, searchQuery));
  }, [projects, searchQuery]);

  if (collapsed) {
    return (
      <aside className="w-14 border-r border-white/5 bg-[#0d0f14]/50 backdrop-blur-xl flex flex-col items-center py-3 z-10 shrink-0 gap-3">
        <button
          onClick={onToggleCollapse}
          className="p-2 text-slate-400 hover:text-white rounded-lg hover:bg-white/5 transition-colors"
          title="Expand sidebar"
        >
          <PanelLeftOpen className="w-4 h-4" />
        </button>
        <button
          onClick={() => setView('new-project')}
          className="p-2 text-slate-400 hover:text-white rounded-lg hover:bg-white/5 transition-colors"
          title="New Project"
        >
          <Plus className="w-5 h-5" />
        </button>
        <div className="w-6 border-t border-white/10" />
        {projects.slice(0, 8).map(p => (
          <button
            key={p.id}
            onClick={() => { setCurrentProject(p.id); setView('home'); }}
            className={`w-9 h-9 rounded-lg flex items-center justify-center text-xs font-bold font-mono transition-all ${
              currentProject === p.id
                ? 'bg-violet-500/20 text-violet-300 border border-violet-500/30'
                : 'text-slate-500 hover:text-slate-300 hover:bg-white/5'
            }`}
            title={p.name}
          >
            {p.name.charAt(0).toUpperCase()}
          </button>
        ))}
      </aside>
    );
  }

  return (
    <aside className="w-60 border-r border-white/5 bg-[#0d0f14]/50 backdrop-blur-xl flex flex-col z-10 shrink-0">
      <div className="p-3 border-b border-white/5 flex justify-between items-center">
        <h2 className="text-[10px] font-outfit font-semibold text-slate-500 tracking-wider uppercase">Workspaces</h2>
        <div className="flex gap-1">
          <button onClick={() => setView('new-project')} className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors" title="Bootstrap Project">
            <Plus className="w-4 h-4" />
          </button>
          <button onClick={onToggleCollapse} className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors" title="Collapse sidebar">
            <PanelLeftClose className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Search */}
      <div className="px-3 py-2 border-b border-white/5">
        <div className="relative">
          <Search className="w-3.5 h-3.5 absolute left-2.5 top-2.5 text-slate-500" />
          <input
            type="text"
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            placeholder="Filter projects..."
            className="w-full bg-black/30 border border-white/5 rounded-md py-1.5 pl-8 pr-2 text-[11px] text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/30"
          />
        </div>
      </div>

      <div className="flex-1 overflow-y-auto py-2">
        {filtered.length === 0 ? (
          <div className="px-4 py-6 text-center text-slate-500 text-xs">
            {searchQuery ? 'No matching projects.' : 'No workspaces configured.'}
          </div>
        ) : (
          filtered.map((p) => (
            <div
              key={p.id}
              onClick={() => { setCurrentProject(p.id); setView('home'); }}
              className={`flex items-center justify-between px-3 py-2 mx-2 rounded-lg cursor-pointer transition-all duration-200 ${
                currentProject === p.id
                  ? 'glass-panel text-white shadow-[0_0_15px_rgba(139,92,246,0.15)]'
                  : 'text-slate-400 hover:bg-white/5 hover:text-slate-200'
              }`}
            >
              <div className="flex items-center gap-2.5 min-w-0">
                <StatusBadge status={p.status} variant="dot" />
                <div className="min-w-0">
                  <div className="text-xs font-medium truncate max-w-[120px]">{p.name}</div>
                  <div className="text-[9px] text-slate-500 font-mono">
                    {statusLabel[p.status] || p.status}
                  </div>
                </div>
              </div>
              <div className="flex items-center gap-2 text-[9px] font-mono text-slate-600 shrink-0">
                <span className="flex items-center gap-0.5">
                  <GitBranch className="w-2.5 h-2.5" />
                  {p.repos}
                </span>
                {p.nodes != null && (
                  <>
                    <span className="text-slate-700">|</span>
                    <span className="flex items-center gap-0.5">
                      <Box className="w-2.5 h-2.5" />
                      {p.nodes}
                    </span>
                  </>
                )}
              </div>
            </div>
          ))
        )}
      </div>

      {/* Keyboard hint */}
      {projects.length > 0 && (
        <div className="px-3 py-2 border-t border-white/5">
          <div className="text-[9px] text-slate-600 font-mono text-center">
            {`⌘1-${Math.min(projects.length, 9)} to jump  ·  ⌘K for palette`}
          </div>
        </div>
      )}
    </aside>
  );
};

export { ProjectRail };
export default ProjectRail;
