import { useState, useMemo } from 'react';
import { Plus, Search, PanelLeftOpen, PanelLeftClose, Sparkles, TerminalSquare } from 'lucide-react';
import { StatusBadge } from './ui/StatusBadge';
import { RailNavItem } from './ui/RailNavItem';
import { ScrollArea } from './ui/ScrollArea';
import { ProjectRailActivity } from './ProjectRailActivity';
import { activityFor, activityTitle, combinedActivity, railProjectLabel, railProjectStatus } from '../lib/projectActivity';
import { useNavigation, useProject, useUIState, useTerminalPanel } from '../context';

function fuzzyMatch(text: string, query: string): boolean {
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  let qi = 0;
  for (let i = 0; i < lower.length && qi < q.length; i++) {
    if (lower[i] === q[qi]) qi++;
  }
  return qi === q.length;
}

const COLLAPSED_PROJECT_LIMIT = 8;

function ProjectRail() {
  const { navigate, view } = useNavigation();
  const { state: { projects, currentProjectId, activityByProject }, dispatch } = useProject();
  const { ui: { sidebarCollapsed }, uiDispatch } = useUIState();
  const { state: terminalState } = useTerminalPanel();
  const terminalCount = terminalState.tabs.length;
  // Terminals blocked on a permission/confirmation gate — the app-wide
  // "needs you" signal (spec `TERMINAL_ACTIVITY` §3). Structurally 0 in
  // Phase 1; lights up once the backend emits `awaiting_approval`.
  const attentionCount = terminalState.tabs.filter(
    (t) => t.activity === 'awaiting_approval',
  ).length;
  const terminalsActive = view.kind === 'terminals';
  const collapsed = sidebarCollapsed;
  const currentProject = currentProjectId;
  const setCurrentProject = (id: string) => { dispatch({ type: 'SET_CURRENT', id }); navigate({ kind: 'home' }); };
  const onToggleCollapse = () => uiDispatch({ type: 'TOGGLE_SIDEBAR' });
  const setView = (v: 'home' | 'new-project') => navigate({ kind: v });

  const [searchQuery, setSearchQuery] = useState('');

  const filtered = useMemo(() => {
    if (!searchQuery.trim()) return projects;
    return projects.filter(p => fuzzyMatch(p.name, searchQuery));
  }, [projects, searchQuery]);

  if (collapsed) {
    const hidden = projects.slice(COLLAPSED_PROJECT_LIMIT);
    const hiddenActivity = combinedActivity(activityByProject, hidden.map(p => p.id));
    const hiddenSummary = activityTitle(hiddenActivity);
    const hiddenTitle = `${hidden.length} more project${hidden.length === 1 ? '' : 's'}`;
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
          title="Bootstrap Project"
        >
          <Plus className="w-5 h-5" />
        </button>
        <button
          onClick={() => navigate({ kind: 'create-project' })}
          className="p-2 text-violet-300 hover:text-white rounded-lg hover:bg-violet-500/10 border border-transparent hover:border-violet-500/30 transition-colors shadow-[0_0_12px_rgba(139,92,246,0.2)]"
          title="New from zero"
        >
          <Sparkles className="w-5 h-5" />
        </button>
        <div className="w-6 border-t border-white/10" />
        {projects.slice(0, COLLAPSED_PROJECT_LIMIT).map(p => (
          <button
            key={p.id}
            onClick={() => { setCurrentProject(p.id); setView('home'); }}
            className={`relative w-9 h-9 rounded-lg flex items-center justify-center text-xs font-bold font-mono transition-all ${
              currentProject === p.id
                ? 'bg-violet-500/20 text-violet-300 border border-violet-500/30'
                : 'text-slate-500 hover:text-slate-300 hover:bg-white/5'
            }`}
            title={p.name}
          >
            {p.name.charAt(0).toUpperCase()}
            <ProjectRailActivity activity={activityFor(activityByProject, p.id)} collapsed />
          </button>
        ))}
        {hidden.length > 0 && (
          <button
            onClick={onToggleCollapse}
            className="relative w-9 h-9 rounded-lg flex items-center justify-center text-[10px] font-mono text-slate-500 hover:text-slate-300 hover:bg-white/5 transition-all"
            title={hiddenSummary ? `${hiddenTitle} — ${hiddenSummary}` : hiddenTitle}
          >
            +{hidden.length}
            <ProjectRailActivity activity={hiddenActivity} collapsed />
          </button>
        )}
        <div className="mt-auto">
          <RailNavItem
            icon={TerminalSquare}
            label="Terminals"
            collapsed
            active={terminalsActive}
            count={terminalCount}
            attentionCount={attentionCount}
            pulse={terminalCount > 0}
            onClick={() => navigate({ kind: 'terminals' })}
          />
        </div>
      </aside>
    );
  }

  return (
    <aside className="w-60 border-r border-white/5 bg-[#0d0f14]/50 backdrop-blur-xl flex flex-col z-10 shrink-0">
      <div className="p-3 border-b border-white/5 flex justify-between items-center">
        <h2 className="text-[10px] font-heading font-semibold text-slate-500 tracking-wider uppercase">Workspaces</h2>
        <div className="flex gap-1">
          <button onClick={() => setView('new-project')} className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors" title="Bootstrap Project">
            <Plus className="w-4 h-4" />
          </button>
          <button
            onClick={() => navigate({ kind: 'create-project' })}
            className="p-1 text-violet-300 hover:text-white rounded hover:bg-violet-500/10 border border-transparent hover:border-violet-500/30 transition-colors shadow-[0_0_10px_rgba(139,92,246,0.2)]"
            title="New from zero"
          >
            <Sparkles className="w-4 h-4" />
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

      <ScrollArea className="flex-1 py-2">
        {filtered.length === 0 ? (
          <div className="px-4 py-6 text-center text-slate-500 text-xs">
            {searchQuery ? 'No matching projects.' : 'No workspaces configured.'}
          </div>
        ) : (
          filtered.map((p) => {
            const activity = activityFor(activityByProject, p.id);
            const derived = railProjectStatus(p.status, activity);
            const summary = activityTitle(activity);
            return (
              <div
                key={p.id}
                onClick={() => { setCurrentProject(p.id); setView('home'); }}
                title={summary ? `${p.name} — ${summary}` : p.name}
                className={`flex items-center justify-between px-3 py-2 mx-2 rounded-lg cursor-pointer transition-all duration-200 ${
                  currentProject === p.id
                    ? 'glass-panel text-white shadow-[0_0_15px_rgba(139,92,246,0.15)]'
                    : 'text-slate-400 hover:bg-white/5 hover:text-slate-200'
                }`}
              >
                <div className="flex items-center gap-2.5 min-w-0">
                  <StatusBadge status={derived} />
                  <div className="min-w-0">
                    <div className="text-xs font-medium truncate max-w-[120px]">{p.name}</div>
                    <div className="text-[9px] text-slate-500 font-mono">
                      {railProjectLabel(derived)}
                    </div>
                  </div>
                </div>
                <ProjectRailActivity activity={activity} />
              </div>
            );
          })
        )}
      </ScrollArea>

      {/* Pinned footer: Terminals entry + keyboard hint */}
      <div className="border-t border-white/5 p-2 space-y-1">
        <RailNavItem
          icon={TerminalSquare}
          label="Terminals"
          active={terminalsActive}
          count={terminalCount}
          attentionCount={attentionCount}
          pulse={terminalCount > 0}
          onClick={() => navigate({ kind: 'terminals' })}
        />
        {projects.length > 0 && (
          <div className="text-[9px] text-slate-600 font-mono text-center pt-1">
            {`⌘1-${Math.min(projects.length, 9)} to jump  ·  ⌘K for palette`}
          </div>
        )}
      </div>
    </aside>
  );
};

export { ProjectRail };
export default ProjectRail;
