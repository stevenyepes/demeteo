import { useEffect, useMemo, useState } from "react";
import TopBar from "./components/TopBar";
import ProjectRail from "./components/ProjectRail";
import { formatError } from "./lib/errors";
import { ErrorBusProvider } from "./lib/errorBus";
import { ErrorToast, ERROR_TOAST_CTA_EVENT } from "./components/ErrorToast";
import EmptyStateCard from "./components/EmptyStateCard";
import NewProjectView from "./components/NewProjectView";
import ProvidersPage from "./components/ProvidersPage";
import RemoteRunInbox from "./components/RemoteRunInbox";
import { Plus, Globe, Box, Zap, Sliders, Settings as SettingsIcon, BookOpen, Server, Terminal as TerminalIcon } from "lucide-react";
import ProjectHome from "./components/ProjectHome";
import ProjectSettings from "./components/ProjectSettings";
import { WorkflowList } from "./components/WorkflowList";
import { WorkflowEditor } from "./components/WorkflowEditor";
import { FeatureDetail } from "./components/FeatureDetail";
import { GateView } from "./components/GateView";
import { CodeEditorView } from "./components/CodeEditorView";
import StartFeatureModal from "./components/StartFeatureModal";
import CreateProjectWizard from "./components/wizard/CreateProjectWizard";
import PreferencesScreen from "./components/PreferencesScreen";
import CommandPalette from "./components/CommandPalette";
import DocsPanel from "./components/DocsPanel";
import type { AppView, Feature, Project, Provider } from "./types";
import { useKeyboardShortcuts } from "./hooks/useKeyboardShortcuts";
import { useLaunchRun } from "./hooks/useLaunchRun";
import { useTauriEvent } from "./hooks/useTauriEvent";
import { MouseNavigationBridge } from "./hooks/useMouseNavigation";
import {
  NavigationProvider, useNavigation,
  ProjectProvider, useProject,
  UIStateProvider, useUIState,
  TerminalPanelProvider,
} from "./context";
import { TerminalsView } from "./components/TerminalsView";
import "./App.css";

// ── Pure helpers (exported for unit tests in src/App.test.tsx) ─────────
//
// These are extracted as standalone functions so the per-spec
// invariants (priority order of Escape, wrap-around cycling, "no-op
// when list is empty") can be pinned down without rendering the full
// app. `AppInner` dispatches the result of `pickEscapeAction` to
// `uiDispatch` / `navigate` / `goBack`; the helpers themselves are
// pure and own no React state.

/**
 * Return the feature that comes AFTER `currentId` in the list, wrapping
 * around. If `currentId` is not in the list, return the first feature.
 * If the list is empty, return `null`. Used by the `onNextFeature`
 * keyboard handler to step through the active project's features.
 */
export function pickNextFeature(features: readonly Feature[], currentId: string | null): Feature | null {
  if (features.length === 0) return null;
  if (currentId === null) return features[0];
  const idx = features.findIndex(f => f.id === currentId);
  if (idx === -1) return features[0];
  return features[(idx + 1) % features.length];
}

/**
 * Return the feature that comes BEFORE `currentId` in the list, wrapping
 * around. If `currentId` is not in the list, return the last feature.
 * If the list is empty, return `null`. Used by the `onPreviousFeature`
 * keyboard handler to step backwards through the active project's
 * features.
 */
export function pickPreviousFeature(features: readonly Feature[], currentId: string | null): Feature | null {
  if (features.length === 0) return null;
  if (currentId === null) return features[features.length - 1];
  const idx = features.findIndex(f => f.id === currentId);
  if (idx === -1) return features[features.length - 1];
  return features[(idx - 1 + features.length) % features.length];
}

/**
 * Minimal UIState slice consumed by `pickEscapeAction`. The helper
 * only needs the open flags (and the editing-provider boolean), so
 * the slice stays narrow and the helper remains decoupled from the
 * full UIState shape. Mirrors the relevant fields of
 * `src/context/UIStateContext.tsx`.
 */
export interface UIStateSlice {
  commandPaletteOpen: boolean;
  docsPanelOpen: boolean;
  isConnectModalOpen: boolean;
  editingProvider: Provider | null;
  startFeatureOpen: boolean;
}

/**
 * Discriminated union of every state mutation a single Escape press
 * can perform. The caller (AppInner) translates each variant into a
 * concrete `uiDispatch` / `navigate` / `goBack` call.
 */
export type EscapeAction =
  | { type: 'close-command-palette' }
  | { type: 'close-docs-panel' }
  | { type: 'close-connect-modal' }
  | { type: 'close-start-feature' }
  | { type: 'close-gate-view'; featureId: string; featureTitle: string }
  | { type: 'navigate-back' };

/**
 * Decide which overlay (if any) a single Escape press should close.
 *
 * Priority order (topmost first, per the implementation spec AC-3):
 *   1. command palette     (ui.commandPaletteOpen)
 *   2. docs panel          (ui.docsPanelOpen)
 *   3. provider connect    (ui.isConnectModalOpen || ui.editingProvider)
 *   4. start-feature modal (ui.startFeatureOpen)
 *   5. gate view overlay   (view.kind === 'detail' && view.gateStepExecutionId)
 *   6. fallback            (navigate back)
 *
 * Per-modal ESC handlers in `CommandPalette`, `StartFeatureModal`,
 * `DocsPanel`, `EnvModal`, `GateView`, etc. are expected to call
 * `event.stopPropagation()` so the global hook only fires once. The
 * notification-bell popover is owned by `NotificationBell` and
 * dismisses itself on the same keypress; the prompt dialog
 * (`FeatureDetail`'s local modal) is handled the same way.
 */
export function pickEscapeAction(ui: UIStateSlice, view: AppView): EscapeAction {
  if (ui.commandPaletteOpen) return { type: 'close-command-palette' };
  if (ui.docsPanelOpen) return { type: 'close-docs-panel' };
  if (ui.isConnectModalOpen || ui.editingProvider !== null) return { type: 'close-connect-modal' };
  if (ui.startFeatureOpen) return { type: 'close-start-feature' };
  if (view.kind === 'detail' && view.gateStepExecutionId) {
    return {
      type: 'close-gate-view',
      featureId: view.featureId,
      featureTitle: view.featureTitle,
    };
  }
  return { type: 'navigate-back' };
}

function AppInner() {
  const { view, navigate, goBack, canGoBack } = useNavigation();
  const { state: proj, dispatch: projDispatch } = useProject();
  const { ui, uiDispatch } = useUIState();

  const { projects, currentProjectId, providers, reposByProject, initialLoadError } = proj;
  // `commandPaletteOpen`, `docsPanelOpen`, `startFeatureOpen`, and
  // `startFeatureWorkflowId` drive the per-overlay render branches
  // below. `isConnectModalOpen` and `editingProvider` are read
  // indirectly via the `ui` object passed to `pickEscapeAction`.
  const { commandPaletteOpen, docsPanelOpen, startFeatureOpen, startFeatureWorkflowId, startFeatureSeed } = ui;

  const currentProject = useMemo(() => projects.find(p => p.id === currentProjectId) ?? null, [projects, currentProjectId]);
  const currentFeatureId: string | null = view.kind === 'detail' ? view.featureId : null;

  // Mirror of the active project's features, kept in memory for
  // `Cmd+G` / `Cmd+Shift+G` cycling. ProjectHome owns its own copy
  // for rendering; this list exists solely to drive the keyboard
  // shortcut and is refreshed on project change + status events.
  const [features, setFeatures] = useState<Feature[]>([]);

  // Bumped by Cmd/Ctrl+T to pop the New-terminal launcher. Scoped to the
  // Terminals view — the launcher only lives there, so the shortcut is a
  // no-op anywhere else rather than yanking the user across the app.
  const [terminalLauncherSignal, setTerminalLauncherSignal] = useState(0);

  // One launch code path for every composer (F28). Pre-seeds the
  // cycling list with the new feature so the user can immediately step
  // through it with Cmd+G; the `feature_status_changed` listener is
  // idempotent so the entry is patched in place, not duplicated.
  const launchRun = useLaunchRun({
    projectId: currentProjectId,
    onLaunched: (feature) =>
      setFeatures(prev => (prev.some(f => f.id === feature.id) ? prev : [...prev, feature])),
  });

  // Refetch the feature list whenever the active project changes.
  // Cancellation flag prevents a slow fetch on the previous project
  // from clobbering the list after the user switches.
  useEffect(() => {
    if (!currentProjectId) {
      setFeatures([]);
      return;
    }
    let cancelled = false;
    const fetchFeatures = async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const list = await invoke<any[]>('fetch_active_features', { projectId: currentProjectId });
        if (cancelled) return;
        const mapped: Feature[] = (list ?? []).map((f: any) => ({
          id: f.id,
          project_id: f.project_id,
          workflow_id: f.workflow_id ?? undefined,
          title: f.title,
          status: f.status,
          total_cost: f.total_cost,
          tokens: f.tokens || 0,
          duration: f.duration,
          created_at: f.created_at,
          agent_kind: f.agent_kind,
          model: f.model,
        }));
        setFeatures(mapped);
      } catch (err) {
        if (!cancelled) {
          console.error("Failed to fetch features for keyboard cycling:", err);
          setFeatures([]);
        }
      }
    };
    fetchFeatures();
    return () => { cancelled = true; };
  }, [currentProjectId]);

  // Keep the in-memory feature list in sync with the orchestrator's
  // status events. New features (just created via `start_feature`)
  // are appended with a placeholder title; the detail view fetches
  // the real title on navigation. Existing features have their
  // status patched in place.
  useTauriEvent<{ feature_id: string; status: string }>('feature_status_changed', ({ feature_id, status }) => {
    setFeatures(prev => {
      const idx = prev.findIndex(f => f.id === feature_id);
      if (idx === -1) {
        return [
          ...prev,
          {
            id: feature_id,
            project_id: currentProjectId ?? '',
            title: 'Feature',
            status,
            total_cost: 0,
            tokens: 0,
            duration: '',
            created_at: Date.now(),
          },
        ];
      }
      const copy = prev.slice();
      copy[idx] = { ...copy[idx], status };
      return copy;
    });
  });

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

  // Gate overlay — fires even when user is on a different view.
  // Uses 'replace' mode so the auto-navigation does not pollute
  // the back stack (per AC-6 in the implementation spec). Also
  // back-fills the feature into the cycling list if it is missing
  // (e.g. the orchestrator resumed a feature the UI has not
  // observed yet).
  useTauriEvent<{ feature_id: string; step_execution_id: string }>('gate_required', ({ feature_id, step_execution_id }) => {
    const featureTitle = view.kind === 'detail' && view.featureId === feature_id ? view.featureTitle : 'Feature Pipeline';
    setFeatures(prev => {
      if (prev.some(f => f.id === feature_id)) return prev;
      return [
        ...prev,
        {
          id: feature_id,
          project_id: currentProjectId ?? '',
          title: featureTitle,
          status: 'pending',
          total_cost: 0,
          tokens: 0,
          duration: '',
          created_at: Date.now(),
        },
      ];
    });
    navigate({ kind: 'detail', featureId: feature_id, featureTitle, gateStepExecutionId: step_execution_id }, 'replace');
  });

  // Initial data load
  useEffect(() => {
    const fetchInitialData = async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');

        // M6.3's "reconcile-on-reopen" desktop notification only fires
        // from inside this call — previously it only ran when the user
        // manually opened the Remote Runs inbox, which defeats the
        // "close the laptop, come back, get told" promise. Fire-and-forget:
        // never blocks the rest of startup, and any newly-actionable run
        // (PR ready/failed/parked/needs-credentials) surfaces as a
        // notification right away instead of waiting for a manual check.
        invoke('remote_reconcile_runs').catch((err) => {
          console.error('Failed to reconcile remote runs on startup:', err);
        });

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
          repoMap[p.id] = reposList.map((r: any) => ({ id: r.id, repo_path: r.repo_path, provider_id: r.provider_id ?? '' }));
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
    onNewFeature: () => {
      if (currentProjectId) {
        uiDispatch({ type: 'OPEN_START_FEATURE' });
      }
    },
    onNewTerminal: () => {
      // Only meaningful on the Terminals view, where the launcher is mounted.
      if (view.kind === 'terminals') {
        setTerminalLauncherSignal((n) => n + 1);
      }
    },
    onToggleSidebar: () => uiDispatch({ type: 'TOGGLE_SIDEBAR' }),
    onCloseCurrentView: () => {
      if (canGoBack) goBack();
    },
    onNextFeature: () => {
      const next = pickNextFeature(features, currentFeatureId);
      if (next) navigate({ kind: 'detail', featureId: next.id, featureTitle: next.title });
    },
    onPreviousFeature: () => {
      const prev = pickPreviousFeature(features, currentFeatureId);
      if (prev) navigate({ kind: 'detail', featureId: prev.id, featureTitle: prev.title });
    },
    onEscape: () => {
      const action = pickEscapeAction(ui, view);
      switch (action.type) {
        case 'close-command-palette':
          uiDispatch({ type: 'SET_COMMAND_PALETTE', open: false });
          break;
        case 'close-docs-panel':
          uiDispatch({ type: 'SET_DOCS_PANEL', open: false });
          break;
        case 'close-connect-modal':
          uiDispatch({ type: 'SET_CONNECT_MODAL', open: false, editing: null });
          break;
        case 'close-start-feature':
          uiDispatch({ type: 'CLOSE_START_FEATURE' });
          break;
        case 'close-gate-view':
          navigate({ kind: 'detail', featureId: action.featureId, featureTitle: action.featureTitle });
          break;
        case 'navigate-back':
          if (canGoBack) goBack();
          break;
      }
    },
    onNavigateProject: (index: number) => {
      const p = projects[index];
      if (p) { projDispatch({ type: 'SET_CURRENT', id: p.id }); navigate({ kind: 'home' }); }
    },
    onOpenTerminalsView: () => navigate({ kind: 'terminals' }),
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
    { id: 'nav-remote-inbox', label: 'Runs', description: 'Every run launched on a remote machine', category: 'action' as const, icon: <Server className="w-4 h-4" />, onSelect: () => navigate({ kind: 'remote-inbox' }) },
    { id: 'nav-settings', label: 'Settings', description: 'Global preferences and machines', category: 'settings' as const, icon: <SettingsIcon className="w-4 h-4" />, onSelect: () => navigate({ kind: 'settings' }) },
    { id: 'nav-docs', label: 'Documentation', description: 'User guide and reference', category: 'settings' as const, icon: <BookOpen className="w-4 h-4" />, onSelect: () => uiDispatch({ type: 'SET_DOCS_PANEL', open: true }) },
    { id: 'nav-shortcuts', label: 'Keyboard Shortcuts', description: 'View available shortcuts', category: 'settings' as const, icon: <Zap className="w-4 h-4" />, onSelect: () => uiDispatch({ type: 'SET_DOCS_PANEL', open: true }) },
    { id: 'nav-terminals', label: 'Terminals', description: 'Open the full-page Terminals view — sessions stay alive as you navigate.', category: 'action' as const, icon: <TerminalIcon className="w-4 h-4" />, onSelect: () => navigate({ kind: 'terminals' }) },
  ], [projects, navigate, projDispatch, uiDispatch]);

  return (
    <div className="flex flex-col h-screen w-screen bg-[#08090c] text-white overflow-hidden font-sans">
      <TopBar connectedProvider={connectedProvider} />
      <div className="flex flex-1 overflow-hidden relative min-h-0">
        <ProjectRail />
        <div className="flex-1 flex flex-col min-h-0 relative">
        <main className="flex-1 flex flex-col relative overflow-hidden bg-[#0a0c10] z-0 min-h-0">

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
                onCreateFromZero={() => navigate({ kind: 'create-project' })}
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

          {view.kind === 'create-project' && <CreateProjectWizard />}

          {view.kind === 'project-settings' && currentProject && <ProjectSettings />}

          {view.kind === 'workflows' && (
            <WorkflowList
              onEdit={(id) => navigate({ kind: 'workflow-editor', workflowId: id })}
              onNew={() => navigate({ kind: 'workflow-editor', workflowId: null })}
              onStartFeature={(wfId) => uiDispatch({ type: 'OPEN_START_FEATURE', workflowId: wfId })}
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

          {view.kind === 'remote-inbox' && <RemoteRunInbox />}

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
              computeType={currentProject.compute_type}
              remoteHost={currentProject.remote_host}
              repositories={reposByProject[currentProjectId] || []}
              defaultWorkflowId={startFeatureWorkflowId}
              seedTitle={startFeatureSeed?.title}
              seedAttachments={startFeatureSeed?.attachments}
              onClose={() => uiDispatch({ type: 'CLOSE_START_FEATURE' })}
              onLaunch={async (params) => {
                const feature = await launchRun(params);
                if (feature) uiDispatch({ type: 'CLOSE_START_FEATURE' });
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
        {/* Keep-mounted, CSS-hidden off-route so the active xterm and all
            backend sessions survive navigation (spec §4.1). */}
        <TerminalsView active={view.kind === 'terminals'} openLauncherSignal={terminalLauncherSignal} />
        </div>
      </div>
    </div>
  );
}

function App() {
  return (
    <ErrorBusProvider>
      <NavigationProvider>
        {/*
          MouseNavigationBridge installs the window-level XButton1 /
          XButton2 listeners that drive `useNavigation().goBack()` /
          `goForward()`. It must be mounted exactly once inside
          NavigationProvider so the useNavigation() call inside the
          hook resolves to a real provider. The bridge returns null
          and contributes no UI of its own.
        */}
        <MouseNavigationBridge />
        <ProjectProvider>
          <UIStateProvider>
            <TerminalPanelProvider>
              <AppInner />
            </TerminalPanelProvider>
            <ErrorToast />
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>
    </ErrorBusProvider>
  );
}

export default App;
