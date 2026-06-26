import { useState, useEffect } from "react";
import TopBar from "./components/TopBar";
import ProjectRail from "./components/ProjectRail";
import { formatError } from "./lib/errors";
import { ErrorBusProvider, useErrorBus } from "./lib/errorBus";
import { ErrorToast, ERROR_TOAST_CTA_EVENT } from "./components/ErrorToast";
import EmptyStateCard from "./components/EmptyStateCard";
import ProviderSettings from "./components/ProviderSettings";
import NewProjectView from "./components/NewProjectView";
import { Plus, Trash2, Globe, Edit2, Box, Zap, Sliders, Settings as SettingsIcon, BookOpen } from "lucide-react";
import ProjectHome from "./components/ProjectHome";
import ProjectSettings from "./components/ProjectSettings";
import { WorkflowList } from "./components/WorkflowList";
import { WorkflowEditor } from "./components/WorkflowEditor";
import { FeatureDetail } from "./components/FeatureDetail";
import { GateView } from "./components/GateView";
import { CodeEditorView } from "./components/CodeEditorView";
import StartFeatureModal from "./components/StartFeatureModal";
import PreferencesScreen from "./components/PreferencesScreen";
import CommandPalette from "./components/CommandPalette";
import DocsPanel from "./components/DocsPanel";
import type { Repository } from "./types";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useTauriEvent } from "./hooks/useTauriEvent";
import "./App.css";

interface Project {
  id: string;
  name: string;
  status: string;
  repos: number;
  nodes: number;
  spend: number;
  tokens: number;
  compute_type?: string;
  remote_host?: string | null;
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

function AppInner() {
  const { reportError } = useErrorBus();
  const [view, setView] = useState<string>('empty-state');
  const [projects, setProjects] = useState<Project[]>([]);
  const [currentProject, setCurrentProject] = useState<string | null>(null);
  const [providers, setProviders] = useState<Provider[]>([]);
  const [isConnectModalOpen, setIsConnectModalOpen] = useState(false);
  const [editingProvider, setEditingProvider] = useState<Provider | null>(null);
  const [activeFeatureId, setActiveFeatureId] = useState<string | null>(null);
  const [selectedWorkflowId, setSelectedWorkflowId] = useState<string | null>(null);
  const [gateStepExecutionId, setGateStepExecutionId] = useState<string | null>(null);
  const [activeFeatureTitle, setActiveFeatureTitle] = useState<string>('Feature Pipeline');
  const [initialLoadError, setInitialLoadError] = useState<string>('');
  const [startFeatureOpen, setStartFeatureOpen] = useState(false);
  const [startFeatureWorkflowId, setStartFeatureWorkflowId] = useState<string | null>(null);
  const [workflowsForModal, setWorkflowsForModal] = useState<Array<{ id: string; name: string; description: string; version: number }>>([]);
  const [reposByProject, setReposByProject] = useState<Record<string, Repository[]>>({});

  const [editorContext, setEditorContext] = useState<{
    machineId: string;
    worktreePath: string;
    branch: string;
    defaultBranch: string;
    initialFile?: string;
  } | null>(null);

  // R7: UX polish state
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [docsPanelOpen, setDocsPanelOpen] = useState(false);

  // Wire the global <ErrorToast> CTAs (e.g. "Open providers" on a `provider`
  // error) into the local router. The toast dispatches CustomEvent; we map
  // each cta to a setView() call. Keeps the toast decoupled from the
  // navigation shape.
  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ cta?: string }>).detail;
      if (!detail || !detail.cta) return;
      switch (detail.cta) {
        case "open-providers":
          setView("providers");
          break;
        case "open-settings":
          setView("settings");
          break;
        case "open-feature":
          setView("detail");
          break;
        case "retry":
        case "view-logs":
        default:
          // No-op: the catch site that originally fired the toast is the
          // one that knows how to retry; "view-logs" will be wired to the
          // log panel in U4.
          break;
      }
    };
    window.addEventListener(ERROR_TOAST_CTA_EVENT, handler);
    return () => window.removeEventListener(ERROR_TOAST_CTA_EVENT, handler);
  }, []);

  useTauriEvent<{ feature_id: string; step_execution_id: string }>('gate_required', ({ feature_id, step_execution_id }) => {
    setGateStepExecutionId(step_execution_id);
    setActiveFeatureId(feature_id);
    setView('detail');
  });

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
        const repoMap: Record<string, Repository[]> = {};
        const mappedProjects: Project[] = await Promise.all(backendProjects.map(async p => {
          let reposList: any[] = [];
          try {
            reposList = await invoke<any[]>('get_repositories_for_project', { projectId: p.id });
          } catch (e) {
            console.error(e);
          }
          repoMap[p.id] = reposList.map((r: any) => ({ id: r.id, repo_path: r.repo_path }));
          return {
            id: p.id,
            name: p.name,
            status: p.status,
            repos: reposList.length,
            nodes: p.nodes,
            spend: p.spend,
            tokens: p.tokens || 0,
            compute_type: p.compute_type,
            remote_host: p.remote_host
          };
        }));
        setProjects(mappedProjects);
        setReposByProject(repoMap);
        if (mappedProjects.length > 0) {
          setCurrentProject(mappedProjects[0].id);
          setView('home');
        }
      } catch (err) {
        // Don't fabricate a "Mock Project (Local)" with a fake
        // github provider — that's how the user ends up trying to
        // bootstrap a workspace that points at github.com with a
        // bogus PAT. Empty state is honest: the user sees the
        // empty-state view and can re-add their real provider.
        const message = formatError(err);
        console.error("Failed to fetch initial data:", err);
        setProviders([]);
        setProjects([]);
        setInitialLoadError(message);
      }
    };
    fetchInitialData();
  }, []);

  // R7: Keyboard shortcuts (hook uses a ref, no useMemo needed)
  useKeyboardShortcuts({
    onOpenCommandPalette: () => setCommandPaletteOpen(true),
    onOpenDocs: () => setDocsPanelOpen(true),
    onOpenSettings: () => setView('settings'),
    onNewProject: () => setView('new-project'),
    onNewFeature: () => setStartFeatureOpen(true),
    onToggleSidebar: () => setSidebarCollapsed(c => !c),
    onEscape: () => {
      if (commandPaletteOpen) setCommandPaletteOpen(false);
      else if (docsPanelOpen) setDocsPanelOpen(false);
      else if (startFeatureOpen) setStartFeatureOpen(false);
    },
    onNavigateProject: (index: number) => {
      const p = projects[index];
      if (p) { setCurrentProject(p.id); setView('home'); }
    },
  });

  // R7: Feature counts for sidebar status dots (reserved for future use)
  useEffect(() => {
    // polling placeholder
  }, [projects]);

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
        tokens: sample.tokens || 0,
        compute_type: sample.compute_type,
        remote_host: sample.remote_host
      };
      setProjects([...projects, sampleProject]);
      setCurrentProject(sampleProject.id);
      setView('home');
    } catch (e) {
      console.error(e);
    }
  };

  const handleProviderConnected = (newProv: Provider) => {
    setProviders(prev => {
      const exists = prev.find(p => p.id === newProv.id);
      if (exists) {
        return prev.map(p => p.id === newProv.id ? newProv : p);
      }
      return [...prev, newProv];
    });
    setIsConnectModalOpen(false);
    setEditingProvider(null);
  };

  return (
    <div className="flex flex-col h-screen w-screen bg-[#08090c] text-white overflow-hidden font-sans">
      <TopBar setView={setView} connectedProvider={connectedProvider} onOpenCommandPalette={() => setCommandPaletteOpen(true)} />
      <div className="flex flex-1 overflow-hidden relative">
        <ProjectRail
          projects={projects.map(p => ({
            ...p,
            repos: p.repos,
            nodes: p.nodes,
          }))}
          currentProject={currentProject}
          setCurrentProject={setCurrentProject}
          setView={setView}
          collapsed={sidebarCollapsed}
          onToggleCollapse={() => setSidebarCollapsed(c => !c)}
        />
        <main className="flex-1 flex flex-col relative overflow-hidden bg-[#0a0c10] z-0">
          {projects.length === 0 && view === 'empty-state' && (
            <>
              {initialLoadError && (
                <div className="mx-8 mt-6 rounded-xl border border-ruby-500/30 bg-ruby-500/5 p-4">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="font-outfit text-sm font-semibold text-ruby-300 uppercase tracking-wider">Failed to load workspace</span>
                  </div>
                  <pre className="font-mono text-xs text-ruby-200/80 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">
{initialLoadError}
                  </pre>
                </div>
              )}
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
            </>
          )}
          {projects.length > 0 && view === 'home' && activeProjectObj && (
            <ProjectHome
              setView={setView}
              activeProject={activeProjectObj}
              setActiveFeatureId={setActiveFeatureId}
              setActiveFeatureTitle={setActiveFeatureTitle}
              setProjects={setProjects}
              sidebarCollapsed={sidebarCollapsed}
            />
          )}
          {view === 'detail' && activeFeatureId && currentProject && (
            <FeatureDetail
              featureId={activeFeatureId}
              projectId={currentProject}
              title={activeFeatureTitle}
              onDecideGate={(stepExecId) => setGateStepExecutionId(stepExecId)}
              onBack={() => setView('home')}
              onOpenEditor={(ctx) => {
                setEditorContext(ctx);
                setView('editor');
              }}
              sidebarCollapsed={sidebarCollapsed}
            />
          )}
          {view === 'editor' && editorContext && (
            <CodeEditorView
              machineId={editorContext.machineId}
              worktreePath={editorContext.worktreePath}
              branch={editorContext.branch}
              defaultBranch={editorContext.defaultBranch}
              featureTitle={activeFeatureTitle}
              initialFile={editorContext.initialFile}
              onBack={() => setView('detail')}
            />
          )}
          {view === 'new-project' && (
            <NewProjectView
              setView={setView}
              setProjects={setProjects}
              setCurrentProjectId={setCurrentProject}
              providers={providers}
              onOpenMachinesSettings={() => setView('settings')}
            />
          )}
          {view === 'project-settings' && activeProjectObj && (
            <ProjectSettings
              setView={setView}
              activeProject={activeProjectObj}
              setProjects={setProjects}
              setCurrentProject={setCurrentProject}
              providers={providers}
            />
          )}
          {view === 'workflows' && (
            <WorkflowList
              onEdit={(id) => {
                setSelectedWorkflowId(id);
                setView('workflow-editor');
              }}
              onNew={() => {
                setSelectedWorkflowId(null);
                setView('workflow-editor');
              }}
              onStartFeature={async (wfId) => {
                // Open the slim StartFeatureModal (Q22). Fetch workflows
                // for the picker once per open.
                try {
                  const { invoke } = await import('@tauri-apps/api/core');
                  const list: any[] = await invoke('workflow_list');
                  setWorkflowsForModal(list.map((w: any) => ({
                    id: w.id,
                    name: w.name,
                    description: w.description,
                    version: w.version,
                  })));
                  setStartFeatureWorkflowId(wfId);
                  setStartFeatureOpen(true);
                } catch (err) {
                  reportError(err);
                }
              }}
            />
          )}
          {view === 'workflow-editor' && (
            <WorkflowEditor
              workflowId={selectedWorkflowId}
              onBack={() => setView('workflows')}
              onSaved={() => setView('workflows')}
            />
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

                        <div className="flex gap-2">
                          <button
                            onClick={() => {
                              setEditingProvider(prov);
                              setIsConnectModalOpen(true);
                            }}
                            className="text-slate-500 hover:text-cyan-400 p-2 rounded-lg hover:bg-white/5 transition-all"
                            title="Edit Provider"
                          >
                            <Edit2 className="w-4 h-4" />
                          </button>
                          <button
                            onClick={async () => {
                              try {
                                const { invoke } = await import('@tauri-apps/api/core');
                                await invoke('delete_provider_instance', { providerId: prov.id });
                                setProviders(providers.filter((p) => p.id !== prov.id));
                              } catch (err) {
                                reportError(err, { kind: "provider" });
                              }
                            }}
                            className="text-slate-500 hover:text-ruby-400 p-2 rounded-lg hover:bg-white/5 transition-all"
                            title="Disconnect Provider"
                          >
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
                  onClose={() => {
                    setIsConnectModalOpen(false);
                    setEditingProvider(null);
                  }}
                />
              )}
            </div>
          )}
          {view === 'settings' && (
            <PreferencesScreen onNavigate={setView} />
          )}
          {gateStepExecutionId && (
            <GateView
              stepExecutionId={gateStepExecutionId}
              onDecisionSubmitted={() => {
                setGateStepExecutionId(null);
                setView('detail');
              }}
              onClose={() => setGateStepExecutionId(null)}
            />
          )}
          {startFeatureOpen && currentProject && activeProjectObj && (
            <StartFeatureModal
              isOpen={startFeatureOpen}
              projectId={currentProject}
              projectName={activeProjectObj.name}
              workflows={workflowsForModal}
              repositories={reposByProject[currentProject] || []}
              defaultWorkflowId={startFeatureWorkflowId}
              onClose={() => setStartFeatureOpen(false)}
              onLaunch={async (params) => {
                try {
                  const { invoke } = await import('@tauri-apps/api/core');
                  const feature: any = await invoke('start_feature', {
                    projectId: currentProject,
                    workflowId: params.workflowId,
                    title: params.title,
                    description: params.description,
                    agentKind: params.agentKind ?? null,
                    model: params.model ?? null,
                    // Per-feature override for `commit_artifacts`.
                    // `undefined` → inherit the project default
                    // (see migration V12 and `StartFeatureModal`).
                    commitArtifacts: params.commitArtifacts ?? null,
                    // Per-run loop budget + per-step agent/model overrides
                    // (migration V13). `null`/empty → inherit defaults.
                    loopIterations: params.loopIterations ?? null,
                    stepOverrides: params.stepOverrides ?? null,
                  });
                  setActiveFeatureId(feature.id);
                  setActiveFeatureTitle(feature.title);
                  setStartFeatureOpen(false);
                  setView('detail');
                } catch (err) {
                  reportError(err);
                }
              }}
            />
          )}

          {/* R7: Command Palette + Docs Panel overlays */}
          <CommandPalette
            isOpen={commandPaletteOpen}
            onClose={() => setCommandPaletteOpen(false)}
            entries={[
              // Project entries
              ...projects.map((p) => ({
                id: `proj-${p.id}`,
                label: p.name,
                description: `${p.repos} repos  ·  ${p.status}`,
                category: 'project' as const,
                icon: <Box className="w-4 h-4" />,
                onSelect: () => { setCurrentProject(p.id); setView('home'); },
              })),
              // Navigation actions
              { id: 'nav-new-project', label: 'New Project', description: 'Bootstrap a new workspace', category: 'action' as const, icon: <Plus className="w-4 h-4" />, onSelect: () => setView('new-project') },
              { id: 'nav-workflows', label: 'Workflows', description: 'View and edit workflow templates', category: 'action' as const, icon: <Sliders className="w-4 h-4" />, onSelect: () => setView('workflows') },
              { id: 'nav-providers', label: 'Providers', description: 'Manage Git hosting connections', category: 'action' as const, icon: <Globe className="w-4 h-4" />, onSelect: () => setView('providers') },
              { id: 'nav-settings', label: 'Settings', description: 'Global preferences and machines', category: 'settings' as const, icon: <SettingsIcon className="w-4 h-4" />, onSelect: () => setView('settings') },
              { id: 'nav-docs', label: 'Documentation', description: 'User guide and reference', category: 'settings' as const, icon: <BookOpen className="w-4 h-4" />, onSelect: () => setDocsPanelOpen(true) },
              { id: 'nav-shortcuts', label: 'Keyboard Shortcuts', description: 'View available shortcuts', category: 'settings' as const, icon: <Zap className="w-4 h-4" />, onSelect: () => setDocsPanelOpen(true) },
            ]}
          />
          <DocsPanel isOpen={docsPanelOpen} onClose={() => setDocsPanelOpen(false)} />
        </main>
      </div>
    </div>
  );
}

function App() {
  return (
    <ErrorBusProvider>
      <AppInner />
      <ErrorToast />
    </ErrorBusProvider>
  );
}

export default App;
