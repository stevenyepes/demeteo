import React from 'react';
import { Plus, LayoutGrid } from 'lucide-react';

interface Project {
  id: string;
  name: string;
  status: string;
  repos: number;
}

interface SidebarProps {
  projects: Project[];
  currentProject: string | null;
  setCurrentProject: (id: string) => void;
  setView: (view: string) => void;
}

const Sidebar: React.FC<SidebarProps> = ({ projects, currentProject, setCurrentProject, setView }) => (
    <aside className="w-64 border-r border-white/5 bg-[#0d0f14]/50 backdrop-blur-xl flex flex-col z-10 shrink-0">
        <div className="p-4 border-b border-white/5 flex justify-between items-center">
            <h2 className="text-xs font-outfit font-semibold text-slate-500 tracking-wider uppercase">Workspaces</h2>
            <div className="flex gap-1">
                <button onClick={() => setView('new-project')} className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors" title="Bootstrap Project">
                    <Plus className="w-4 h-4" />
                </button>
                <button onClick={() => setView('home')} className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors" title="Dashboard home">
                    <LayoutGrid className="w-4 h-4" />
                </button>
            </div>
        </div>
        <div className="flex-1 overflow-y-auto py-2">
            {projects.length === 0 ? (
                <div className="px-4 py-6 text-center text-slate-500 text-sm">
                    No workspaces configured.
                </div>
            ) : (
                projects.map(p => (
                    <div
                        key={p.id}
                        onClick={() => {
                            setCurrentProject(p.id);
                            setView('home');
                        }}
                        className={`flex items-center justify-between px-4 py-2.5 mx-2 rounded-lg cursor-pointer transition-all duration-200 ${currentProject === p.id
                                ? 'glass-panel text-white shadow-[0_0_15px_rgba(139,92,246,0.15)]'
                                : 'text-slate-400 hover:bg-white/5 hover:text-slate-200'
                            }`}
                    >
                        <div className="flex items-center gap-3">
                            <div className={`w-2 h-2 rounded-full ${p.status === 'active' ? 'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]' :
                                    p.status === 'error' ? 'bg-ruby-500 shadow-[0_0_8px_rgba(239,68,68,0.8)]' : 'bg-slate-600'
                                }`} />
                            <span className="text-sm font-medium truncate w-32">{p.name}</span>
                        </div>
                        <span className="text-[10px] font-mono opacity-50">{p.repos} Repos</span>
                    </div>
                ))
            )}
        </div>
    </aside>
);

export default Sidebar;
