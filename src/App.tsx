import { useEffect, useMemo } from "react";
import TopBar from "./components/TopBar";
import ProjectRail from "./components/ProjectRail";
import { formatError } from "./lib/errors";
import { ErrorBusProvider, useErrorBus } from "./lib/errorBus";
import { ErrorToast, ERROR_TOAST_CTA_EVENT } from "./components/ErrorToast";
import EmptyStateCard from "./components/EmptyStateCard";
import NewProjectView from "./components/NewProjectView";
import ProvidersPage from "./components/ProvidersPage";
import { Plus, Globe, Box, Zap, Sliders, Settings as SettingsIcon, BookOpen } from "lucide-react";
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
import type { Project, Provider } from "./types";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useTauriEvent } from "./hooks/useTauriEvent";
import {
  NavigationProvider, useNavigation,
  ProjectProvider, useProject,
  UIStateProvider, useUIState,
} from "./context";
import "./App.css";

function AppInner() {
  const { reportError } = useErrorBus();
  const { view, navigate } = useNavigation();
  const { state: proj, dispatch: projDispatch } = useProject();
  const { ui, uiDispatch } = useUIState();

  const { projects, currentProjectId, providers, reposByProject, workflowsForModal, initialLoadError } = proj;
  const { commandPaletteOpen, docsPanelOpen, startFeatureOpen, startFeatureWorkflowId } = ui;

  const currentProject = useMemo(() => projects.find(p => p.id === currentProjectId) ?? null, [projects, currentProjectId]);

  // Map CTA events from ErrorToast into navigation
  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ cta?: string }>).detail;
      if (!detail?.cta) return;
      switch (detail.cta) {
        case "open-providers": navigate({ kind: 'providers' }); break;
        case "open-settings": navigate({ kind: 'settings' }); break;
        case "open-feature": navigate({ kind: view.kind === 'detail' ? 'detail' : 'home', ...(view.kind === 'detail' ? view : {}) } as any); break;
      }
    };
    window.addEventListener(ERROR_TOAST_CTA_EVENT, handler);
    return () => window.removeEventListener(ERROR_TOAST_CTA_EVENT, handler);
  }, [navigate, view]);

  // Gate overlay — fires even when user is on a different view
  useTauriEvent<{ feature_id: string; step_execution_id: string }>('gate_required', ({ feature_id, step_execution_id }) => {
    const featureTitle = view.kind === 'detail' && view.featureId === feature_id ? view.featureTitle : 'Feature Pipeline';
    navigate({ kind: 'detail', featureId: feature_id, featureTitle, gateStepExecutionId: step_execution_id });
  });

  // Initial data load
  useEffect(() => {
    const fetchInitialData = async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');

        const backendProviders: any[] = await invoke('list_provider_instances');
        const mappedProviders: Provider[] = backendProviders.map(p => ({
          id: p.id, type: p.kind, name: p.kind, host: p.host,
          pat: 'hidden', username: p.username, avatarUrl: p.avatar_url,
        }));
        projDispatch({ type: 'SET_PROVIDERS', providers: mappedProviders });

        const backendProjects: any[] = await invoke('get_projects');
        const repoMap: Record<string, import('./types').Repository[]> = {};
        const mappedProjects: Project[] = await Promise.all(backendProjects.map(async p => {
          let reposList: any[] = [];
          try { reposList = await invoke<any[]>('get_repositories_for_project', { projectId: p.id }); } catch {}
          repoMap[p.id] = reposList.map((r: any) => ({ id: r.id, repo_path: r.repo_path }));
          return {
            id: p.id, name: p.name, status: p.status,
            repos: reposList.length, nodes: p.nodes, spend: p.spend,
            tokens: p.tokens || 0, compute_type: p.compute_type, remote_host: p.remote_host,
          };
        }));

        projDispatch({ type: 'LOAD_PROJECTS', projects: mappedProjects, reposByProject: repoMap });
        if (mappedProjects.length > 0) {
          projDispatch({ type: 'SET_CURRENT', id: mappedProjects[0].id });
          navigate({ kind: 'home' });
        }
      } catch (err) {
        console.error("Failed to fetch initial data:", err);
        projDispatch({ type: 'SET_ERROR', error: formatError(err) });
        projDispatch({ type: 'SET_PROVIDERS', providers: [] });
        projDispatch({ type: 'LOAD_PROJECTS', projects: [], reposByProject: {} });
      }
    };
    fetchInitialData();
  }, []);

  // Navigate to empty-state when projects list empties
  useEffect(() => {
    if (projects.length === 0 && view.kind === 'home') {
      navigate({ kind: 'empty-state' });
    }
  }, [projects, view.kind]);

  useKeyboardShortcuts({
    onOpenCommandPalette: () => uiDispatch({ type: 'SET_COMMAND_PALETTE', open: true }),
    onOpenDocs: () => uiDispatch({ type: 'SET_DOCS_PANEL', open: true }),
    onOpenSettings: () => navigate({ kind: 'settings' }),
    onNewProject: () => navigate({ kind: 'new-project' }),
    onNewFeature: () => uiDispatch({ type: 'OPEN_START_FEATURE' }),
    onToggleSidebar: () => uiDispatch({ type: 'TOGGLE_SIDEBAR' }),
    onEscape: () => {
      if (commandPaletteOpen) uiDispatch({ type: 'SET_COMMAND_PALETTE', open: false });
      else if (docsPanelOpen) uiDispatch({ type: 'SET_DOCS_PANEL', open: false });
      else if (startFeatureOpen) uiDispatch({ type: 'CLOSE_START_FEATURE' });
    },
    onNavigateProject: (index: number) => {
      const p = projects[index];
      if (p) { projDispatch({ type: 'SET_CURRENT', id: p.id }); navigate({ kind: 'home' }); }
    },
  });

  const handleSeedSample = async () => {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const sample: any = await invoke('seed_sample_project');
      const sampleProject: Project = {
        id: sample.id, name: sample.name, status: sample.status,
        repos: 2, nodes: sample.nodes, spend: sample.spend,
        tokens: sample.tokens || 0, compute_type: sample.compute_type, remote_host: sample.remote_host,
      };
      projDispatch({ type: 'ADD_PROJECT', project: sampleProject });
      projDispatch({ type: 'SET_CURRENT', id: sampleProject.id });
      navigate({ kind: 'home' });
    } catch (e) { console.error(e); }
  };

  const connectedProvider = providers[0] ?? null;

  const commandPaletteEntries = useMemo(() => [
    ...projects.map((p) => ({
      id: `proj-${p.id}`,
      label: p.name,
      description: `${p.repos} repos · ${p.status}`,
      category: 'project' as const,
      icon: <Box className="w-4 h-4" />,
      onSelect: () => { projDispatch({ type: 'SET_CURRENT', id: p.id }); navigate({ kind: 'home' }); },
    })),
    { id: 'nav-new-project', label: 'New Project', description: 'Bootstrap a new workspace', category: 'action' as const, icon: <Plus className="w-4 h-4" />, onSelect: () => navigate({ kind: 'new-project' }) },
    { id: 'nav-workflows', label: 'Workflows', description: 'View and edit workflow templates', category: 'action' as const, icon: <Sliders className="w-4 h-4" />, onSelect: () => navigate({ kind: 'workflows' }) },
    { id: 'nav-providers', label: 'Providers', description: 'Manage Git hosting connections', category: 'action' as const, icon: <Globe className="w-4 h-4" />, onSelect: () => navigate({ kind: 'providers' }) },
    { id: 'nav-settings', label: 'Settings', description: 'Global preferences and machines', category: 'settings' as const, icon: <SettingsIcon className="w-4 h-4" />, onSelect: () => navigate({ kind: 'settings' }) },
    { id: 'nav-docs', label: 'Documentation', description: 'User guide and reference', category: 'settings' as const, icon: <BookOpen className="w-4 h-4" />, onSelect: () => uiDispatch({ type: 'SET_DOCS_PANEL', open: true }) },
    { id: 'nav-shortcuts', label: 'Keyboard Shortcuts', description: 'View available shortcuts', category: 'settings' as const, icon: <Zap className="w-4 h-4" />, onSelect: () => uiDispatch({ type: 'SET_DOCS_PANEL', open: true }) },
  ], [projects, navigate, projDispatch, uiDispatch]);

  return (
    <div className="flex flex-col h-screen w-screen bg-[#08090c] text-white overflow-hidden font-sans">
      <TopBar connectedProvider={connectedProvider} />
      <div className="flex flex-1 overflow-hidden relative">
        <ProjectRail />
        <main className="flex-1 flex flex-col relative overflow-hidden bg-[#0a0c10] z-0">

          {/* empty-state */}
          {view.kind === 'empty-state' && (
            <>
              {initialLoadError && (
                <div className="mx-8 mt-6 rounded-xl border border-ruby-500/30 bg-ruby-500/5 p-4">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="font-outfit text-sm font-semibold text-ruby-300 uppercase tracking-wider">Failed to load workspace</span>
                  </div>
                  <pre className="font-mono text-xs text-ruby-200/80 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">{initialLoadError}</pre>
                </div>
              )}
              <EmptyStateCard
                onSeedSample={handleSeedSample}
                onConnectProviders={() => { navigate({ kind: 'providers' }); uiDispatch({ type: 'SET_CONNECT_MODAL', open: true }); }}
                onSyncWorktrees={() => navigate({ kind: 'new-project' })}
                onDeployAgents={() => navigate({ kind: 'workflows' })}
              />
            </>
          )}

          {view.kind === 'home' && currentProject && <ProjectHome />}

          {view.kind === 'detail' && <FeatureDetail />}

          {view.kind === 'editor' && (
            <CodeEditorView
              machineId={view.editorContext.machineId}
              worktreePath={view.editorContext.worktreePath}
              branch={view.editorContext.branch}
              defaultBranch={view.editorContext.defaultBranch}
              featureTitle={view.featureTitle}
              initialFile={view.editorContext.initialFile}
              onBack={() => navigate({ kind: 'detail', featureId: view.featureId, featureTitle: view.featureTitle })}
            />
          )}

          {view.kind === 'new-project' && <NewProjectView />}

          {view.kind === 'project-settings' && currentProject && <ProjectSettings />}

          {view.kind === 'workflows' && (
            <WorkflowList
              onEdit={(id) => navigate({ kind: 'workflow-editor', workflowId: id })}
              onNew={() => navigate({ kind: 'workflow-editor', workflowId: null })}
              onStartFeature={async (wfId) => {
                try {
                  const { invoke } = await import('@tauri-apps/api/core');
                  const list: any[] = await invoke('workflow_list');
                  projDispatch({
                    type: 'SET_WORKFLOWS_FOR_MODAL',
                    workflows: list.map((w: any) => ({ id: w.id, name: w.name, description: w.description, version: w.version })),
                  });
                  uiDispatch({ type: 'OPEN_START_FEATURE', workflowId: wfId });
                } catch (err) { reportError(err); }
              }}
            />
          )}

          {view.kind === 'workflow-editor' && (
            <WorkflowEditor
              workflowId={view.workflowId}
              onBack={() => navigate({ kind: 'workflows' })}
              onSaved={() => navigate({ kind: 'workflows' })}
            />
          )}

          {view.kind === 'providers' && <ProvidersPage />}

          {view.kind === 'settings' && <PreferencesScreen />}

          {/* Gate overlay — rendered on top of detail view */}
          {view.kind === 'detail' && view.gateStepExecutionId && (
            <GateView
              stepExecutionId={view.gateStepExecutionId}
              onDecisionSubmitted={() => navigate({ kind: 'detail', featureId: view.featureId, featureTitle: view.featureTitle })}
              onClose={() => navigate({ kind: 'detail', featureId: view.featureId, featureTitle: view.featureTitle })}
            />
          )}

          {/* Start Feature modal */}
          {startFeatureOpen && currentProjectId && currentProject && (
            <StartFeatureModal
              isOpen={startFeatureOpen}
              projectId={currentProjectId}
              projectName={currentProject.name}
              workflows={workflowsForModal}
              repositories={reposByProject[currentProjectId] || []}
              defaultWorkflowId={startFeatureWorkflowId}
              onClose={() => uiDispatch({ type: 'CLOSE_START_FEATURE' })}
              onLaunch={async (params) => {
                try {
                  const { invoke } = await import('@tauri-apps/api/core');
                  const feature: any = await invoke('start_feature', {
                    projectId: currentProjectId,
                    workflowId: params.workflowId,
                    title: params.title,
                    description: params.description,
                    agentKind: params.agentKind ?? null,
                    model: params.model ?? null,
                    commitArtifacts: params.commitArtifacts ?? null,
                    loopIterations: params.loopIterations ?? null,
                    stepOverrides: params.stepOverrides ?? null,
                  });
                  uiDispatch({ type: 'CLOSE_START_FEATURE' });
                  navigate({ kind: 'detail', featureId: feature.id, featureTitle: feature.title });
                } catch (err) { reportError(err); }
              }}
            />
          )}

          <CommandPalette
            isOpen={commandPaletteOpen}
            onClose={() => uiDispatch({ type: 'SET_COMMAND_PALETTE', open: false })}
            entries={commandPaletteEntries}
          />
          <DocsPanel isOpen={docsPanelOpen} onClose={() => uiDispatch({ type: 'SET_DOCS_PANEL', open: false })} />
        </main>
      </div>
    </div>
  );
}

function App() {
  return (
    <ErrorBusProvider>
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <AppInner />
            <ErrorToast />
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>
    </ErrorBusProvider>
  );
}

export default App;
