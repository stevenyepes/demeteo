import { useState, useEffect } from "react";
import TopBar from "./components/TopBar";
import Sidebar from "./components/Sidebar";
import EmptyStateCard from "./components/EmptyStateCard";
import ProviderSettings from "./components/ProviderSettings";
import NewProjectView from "./components/NewProjectView";
import { Plus, Trash2, Globe } from "lucide-react";
import ProjectHome from "./components/ProjectHome";
import "./App.css";

interface Project {
  id: string;
  name: string;
  status: string;
  repos: number;
  nodes: number;
  spend: number;
}

interface Provider {
  id: string;
  type: string;
  name: string;
  host: string;
  pat: string;
  username: string;
  avatarUrl: string;
}

function App() {
  const [view, setView] = useState<string>('empty-state');
  const [projects, setProjects] = useState<Project[]>([]);
  const [currentProject, setCurrentProject] = useState<string | null>(null);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [isConnectModalOpen, setIsConnectModalOpen] = useState(false);
  const [activeFeatureId, setActiveFeatureId] = useState<string | null>(null);

  useEffect(() => {
    // Fetch initial data
    const fetchInitialData = async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        
        const backendProviders: any[] = await invoke('list_provider_instances');
        const mappedProviders: Provider[] = backendProviders.map(p => ({
          id: p.id,
          type: p.kind,
          name: p.kind, // UI allows naming, but we don't store it in ProviderInstance schema currently, defaulting to kind.
          host: p.host,
          pat: 'hidden',
          username: p.username,
          avatarUrl: p.avatar_url
        }));
        setProviders(mappedProviders);

        const backendProjects: any[] = await invoke('get_projects');
        const mappedProjects: Project[] = backendProjects.map(p => ({
          id: p.id,
          name: p.name,
          status: p.status,
          repos: 1, // Scaffolded
          nodes: p.nodes,
          spend: p.spend
        }));
        setProjects(mappedProjects);
        if (mappedProjects.length > 0) {
          setCurrentProject(mappedProjects[0].id);
          setView('home');
        }
      } catch (err) {
        console.error("Failed to fetch initial data", err);
      }
    };
    fetchInitialData();
  }, []);

  const connectedProvider = providers[0] || null;
  const activeProjectObj = projects.find(p => p.id === currentProject);

  useEffect(() => {
    if (projects.length === 0 && view === 'home') {
      setView('empty-state');
    }
  }, [projects, view]);

  const handleSeedSample = async () => {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const sample: any = await invoke('seed_sample_project');
      const sampleProject: Project = {
        id: sample.id,
        name: sample.name,
        status: sample.status,
        repos: 2,
        nodes: sample.nodes,
        spend: sample.spend,
      };
      setProjects([...projects, sampleProject]);
      setCurrentProject(sampleProject.id);
      setView('home');
    } catch (e) {
      console.error(e);
    }
  };

  const handleProviderConnected = (newProv: Provider) => {
    setProviders([...providers, newProv]);
    setIsConnectModalOpen(false);
  };

  return (
    <div className="flex flex-col h-screen w-screen bg-[#08090c] text-white overflow-hidden font-sans">
      <TopBar setView={setView} connectedProvider={connectedProvider} />
      <div className="flex flex-1 overflow-hidden relative">
        <Sidebar 
          projects={projects} 
          currentProject={currentProject} 
          setCurrentProject={setCurrentProject}
          setView={setView}
        />
        <main className="flex-1 flex flex-col relative overflow-hidden bg-[#0a0c10] z-0">
          {projects.length === 0 && view === 'empty-state' && (
            <EmptyStateCard 
              onSeedSample={handleSeedSample}
              onConnectProviders={() => {
                setView('providers');
                setIsConnectModalOpen(true);
              }}
              onSyncWorktrees={() => {
                setView('new-project');
              }}
              onDeployAgents={() => {
                setView('workflows');
              }}
            />
          )}
          {projects.length > 0 && view === 'home' && activeProjectObj && (
            <ProjectHome
              setView={setView}
              activeProject={activeProjectObj}
              setActiveFeatureId={setActiveFeatureId}
            />
          )}
          {view === 'detail' && (
            <div className="p-8 flex items-center justify-center h-full">
               <div className="text-center">
                 <h1 className="text-2xl text-slate-300 font-outfit mb-2">Feature Detail</h1>
                 <p className="text-slate-500 font-mono text-sm">Feature detail monitoring coming in Story 5 (ID: {activeFeatureId})</p>
               </div>
            </div>
          )}
          {view === 'new-project' && (
            <NewProjectView
              setView={setView}
              setProjects={setProjects}
              setCurrentProjectId={setCurrentProject}
              providers={providers}
            />
          )}
          {view === 'workflows' && (
            <div className="p-8 flex items-center justify-center h-full">
               <div className="text-center">
                 <h1 className="text-2xl text-slate-300 font-outfit mb-2">Workflows Catalog</h1>
                 <p className="text-slate-500 font-mono text-sm">Workflow authoring coming in Story 8</p>
               </div>
            </div>
          )}
          {view === 'providers' && (
            <div className="flex-1 overflow-y-auto p-8 relative flex flex-col justify-start max-w-4xl mx-auto w-full">
              <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-cyan-600/5 rounded-full blur-[120px] pointer-events-none"></div>

              <div className="flex justify-between items-center mb-8 border-b border-white/5 pb-4 z-10">
                <div>
                  <h1 className="text-2xl font-outfit font-bold text-white mb-2">Source Providers</h1>
                  <p className="text-sm text-slate-400">Manage Git hosting endpoints for cloning repositories and creating merge requests.</p>
                </div>
                <button
                  onClick={() => setIsConnectModalOpen(true)}
                  className="bg-cyan-600 hover:bg-cyan-500 text-white font-medium text-sm px-4 py-2 rounded-lg transition-all shadow-[0_0_15px_rgba(6,182,212,0.3)] flex items-center gap-2"
                >
                  <Plus className="w-4 h-4" /> Connect Provider
                </button>
              </div>

              <div className="space-y-4 z-10">
                {providers.length === 0 ? (
                  <div className="glass-panel p-12 text-center flex flex-col items-center justify-center">
                    <Globe className="w-12 h-12 text-slate-500 mb-4 animate-pulse" />
                    <h3 className="text-lg font-outfit font-semibold text-white mb-2">No Providers Mapped</h3>
                    <p className="text-sm text-slate-400 max-w-md mb-6">Connect your GitHub or GitLab workspace to enable repository cloning, branch management, and automatic pull requests.</p>
                    <button
                      onClick={() => setIsConnectModalOpen(true)}
                      className="bg-cyan-600 hover:bg-cyan-500 text-white font-medium text-sm px-5 py-2.5 rounded-lg transition-all shadow-[0_0_15px_rgba(6,182,212,0.3)]"
                    >
                      Connect Your First Account
                    </button>
                  </div>
                ) : (
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    {providers.map((prov) => (
                      <div key={prov.id} className="glass-panel p-5 flex items-start justify-between border-l-2 border-l-cyan-500 hover:border-l-cyan-400 transition-all">
                        <div className="flex gap-4">
                          {prov.avatarUrl ? (
                            <img
                              src={prov.avatarUrl}
                              alt={prov.username}
                              className="w-12 h-12 rounded-full object-cover border border-white/10"
                            />
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

                        <button
                          onClick={() => setProviders(providers.filter((p) => p.id !== prov.id))}
                          className="text-slate-500 hover:text-ruby-400 p-2 rounded-lg hover:bg-white/5 transition-all"
                          title="Disconnect Provider"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {isConnectModalOpen && (
                <ProviderSettings
                  onConnected={handleProviderConnected}
                  onClose={() => setIsConnectModalOpen(false)}
                />
              )}
            </div>
          )}
          {view === 'settings' && (
            <div className="p-8 flex items-center justify-center h-full">
               <div className="text-center">
                 <h1 className="text-2xl text-slate-300 font-outfit mb-2">Global Settings</h1>
                 <p className="text-slate-500 font-mono text-sm">Settings coming in Story 9</p>
               </div>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}

export default App;
