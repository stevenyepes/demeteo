import { useCallback, useState } from 'react';
import { RefreshCw, ShieldAlert } from 'lucide-react';
import type { AppView } from '../../types';
import type { NavigationMode } from '../../context/NavigationContext';
import { DEFAULT_DENSITY, type Density } from '../../lib/density';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import { useNavigation, useProject, useUIState } from '../../context';
import { ArtifactModal } from '../ArtifactModal';
import { RunViewToggle } from '../RunViewToggle';
import { defaultInspectorWidth, pickInspectorLayout } from '../runLayout';
import { DensityToggle } from '../ui/DensityToggle';
import { useRunColumnLayout } from '../useRunColumnLayout';
import { AttachmentPreviewModal } from './AttachmentPreviewModal';
import { AttachmentsPanel } from './AttachmentsPanel';
import { FeatureHeader } from './FeatureHeader';
import { FeatureStatusBanners } from './FeatureStatusBanners';
import { GateStrip } from './GateStrip';
import { InitialPromptPanel } from './InitialPromptPanel';
import { ReplayModal } from './ReplayModal';
import { RunGraphPanel } from './RunGraphPanel';
import { RunMetaColumn } from './RunMetaColumn';
import { RunPanes } from './RunPanes';
import { StepInspector } from './StepInspector';
import { StepTimeline } from './StepTimeline';
import { useAgentStream } from './useAgentStream';
import { useArtifactSelection } from './useArtifactSelection';
import { useAttachmentPreview } from './useAttachmentPreview';
import { useBootstrapPhases } from './useBootstrapPhases';
import { useFeatureMr } from './useFeatureMr';
import { useFeatureRun } from './useFeatureRun';
import { useGateCardScroll } from './useGateCardScroll';
import { useHarnessOverrides } from './useHarnessOverrides';
import { useHeaderCollapse } from './useHeaderCollapse';
import { useRemoteRun } from './useRemoteRun';
import { useRerunActions } from './useRerunActions';
import { useRunGraph } from './useRunGraph';
import { useStepSelection } from './useStepSelection';
import { useWorktreeRouting } from './useWorktreeRouting';

type DetailView = Extract<AppView, { kind: 'detail' }>;

interface FeatureDetailViewProps {
  view: DetailView;
  navigate: (view: AppView, mode?: NavigationMode) => void;
}

/**
 * The narrowing is the whole job of this component: `FeatureDetailView` cannot
 * mount without a detail view, so none of its ~20 hooks is reachable
 * conditionally. Guarding inside the body instead reads as safe only while
 * `App.tsx` mounts this for `kind: 'detail'` alone — the first caller that keeps
 * it mounted across a view change gets a hooks-order crash (audit F17).
 */
export function FeatureDetail() {
  const { view, navigate } = useNavigation();
  if (view.kind !== 'detail') return null;
  return <FeatureDetailView view={view} navigate={navigate} />;
}

function FeatureDetailView({ view, navigate }: FeatureDetailViewProps) {
  const { state: { currentProjectId, projects } } = useProject();
  const { ui: { sidebarCollapsed: _sidebarCollapsed } } = useUIState();
  // The legacy `AgentTerminalDrawer` mount consumed `sidebarCollapsed`
  // to size its drawer; the panel-mounted surface does not need it.
  // We keep the field on `UIStateSlice` for back-compat with the
  // `pickEscapeAction` helper exported from `App.tsx`.
  void _sidebarCollapsed;

  const { featureId } = view;
  const projectId = currentProjectId ?? undefined;
  const currentProject = projects.find(p => p.id === currentProjectId) ?? null;

  const overrides = useHarnessOverrides();
  const run = useFeatureRun({
    featureId,
    projectId,
    initialTitle: view.featureTitle || 'Feature Pipeline',
    overrides,
  });
  const bootstrap = useBootstrapPhases({
    featureId,
    featureStatus: run.featureStatus,
    anyStepStarted: run.anyStepStarted,
  });
  const remote = useRemoteRun({
    featureId,
    reload: run.reload,
    upsertBootstrapPhase: bootstrap.upsertBootstrapPhase,
  });
  const attachments = useAttachmentPreview(featureId);
  const stream = useAgentStream(featureId);
  const artifact = useArtifactSelection(run.steps);
  const rerun = useRerunActions({
    featureId,
    remoteRun: remote.remoteRun,
    refreshRemoteRun: remote.refreshRemoteRun,
    reload: run.reload,
    setFeatureStatus: run.setFeatureStatus,
    overrides,
  });
  const selection = useStepSelection({ view, steps: run.steps, navigate });
  const graph = useRunGraph({
    featureId,
    featureTitle: run.featureTitle,
    steps: run.steps,
    navigate,
    startReplay: rerun.startReplay,
    selectedNodeId: selection.selectedNodeId,
    toggleNode: selection.toggleStep,
  });
  const mr = useFeatureMr({
    featureId,
    projectId,
    status: run.status,
    reload: run.reload,
    navigate,
  });
  const routing = useWorktreeRouting({
    featureId,
    featureTitle: run.featureTitle,
    projectId,
    remoteRun: remote.remoteRun,
    navigate,
  });
  const stepCardRefs = useGateCardScroll(view.gateStepExecutionId, run.steps.length);

  /** Measuring the column, measuring the chrome above the graph, and turning
   *  both into verdicts lives in `useRunColumnLayout` — this component only
   *  hands out the refs and renders what comes back. */
  const {
    setRunColumnEl,
    runColumnEl,
    setMetaChromeEl,
    setToggleChromeEl,
    runColumnSize,
    runLayout,
    graphBoxPx,
    surfaceBoxPx,
  } = useRunColumnLayout(graph.graphDef);
  const headerCollapsed = useHeaderCollapse(runColumnEl);

  /** `null` until the user drags, and then theirs for good. Until then the
   *  opening width tracks the measured column, so a window resized before
   *  anyone touched the divider still opens at a sensible proportion; after a
   *  drag nothing re-derives it, which is the discipline `runLayout.ts`'s doc
   *  asks of its callers. Phase 6 owns persisting it — this outlives a resize,
   *  not a remount. */
  const [draggedInspectorWidth, setDraggedInspectorWidth] = useState<number | null>(null);
  /** Phase 6 persists this through `get_app_session`/`set_app_session`; until
   *  then it is the session's, not the user's. */
  const [density, setDensity] = useState<Density>(DEFAULT_DENSITY);
  const inspectorLayout = pickInspectorLayout(runColumnSize);
  const inspectorWidth = draggedInspectorWidth ?? defaultInspectorWidth(runColumnSize);

  useTauriEvent<{ feature_id: string; step_execution_id: string }>('gate_required', ({ feature_id, step_execution_id }) => {
    if (feature_id === featureId) {
      // Force a refetch on the very next event tick so the timeline
      // re-derives the "active" chip immediately rather than waiting
      // for the next 1 Hz heartbeat. Prevents a stale
      // `awaiting_gate` chip lingering alongside the new gate card.
      run.reload();
      navigate({ kind: 'detail', featureId, featureTitle: run.featureTitle, gateStepExecutionId: step_execution_id });
    }
  });

  const decideGate = useCallback(
    (stepExecutionId: string) =>
      navigate({ kind: 'detail', featureId, featureTitle: run.featureTitle, gateStepExecutionId: stepExecutionId }),
    [navigate, featureId, run.featureTitle],
  );

  const deselectStep = useCallback(() => selection.selectStep(null), [selection.selectStep]);

  // The unified feed the inspector reads: local runs push it through
  // `useRunEvents`; remote runs fill `remoteRunEvents` from the poll above.
  const panelRunEvents = remote.remoteRun ? remote.remoteRunEvents : graph.localRunEvents;

  const { graphDef } = graph;

  /** The one inspector, docked beside whichever run surface is showing
   *  (UI_REDESIGN_PLAN §3.1). It subscribes to the live stream itself — see
   *  `StepInspector`'s header for why that subscription may not live here. */
  const inspector = (
    <StepInspector
      className="h-full"
      featureId={featureId}
      target={selection.target}
      graphDef={graphDef}
      statusByNode={graph.runStatusByNode}
      runEvents={panelRunEvents}
      streamStore={stream.store}
      harnessBaseline={run.harnessBaseline}
      overrides={overrides}
      onDeselect={deselectStep}
      onOpenEditorForPath={routing.openEditorForPath}
      onOpenArtifact={artifact.openArtifact}
      onRetry={rerun.handleRetryStep}
      onReplay={graph.startReplayFromInspector}
      onStop={rerun.handleStopStep}
      onDecideGate={decideGate}
    />
  );

  const runSurface =
    graph.graphMode && graphDef ? (
      <RunGraphPanel
        definition={graphDef}
        statusByNode={graph.runStatusByNode}
        highlightedNodeIds={rerun.replayPreviewNodes}
        selectedNodeId={selection.selectedNodeId}
        onNodeActivate={graph.onNodeActivate}
      />
    ) : (
      <StepTimeline
        steps={run.steps}
        remoteRun={remote.remoteRun}
        remoteMachineName={remote.remoteMachineName}
        hasBootstrapPhases={bootstrap.bootstrapPhases.size > 0}
        gateStepExecutionId={view.gateStepExecutionId}
        stepCardRefs={stepCardRefs}
        selectedStepId={selection.selectedExecutionId}
        density={density}
        onSelect={selection.selectStep}
        onDecideGate={decideGate}
      />
    );

  /** The graph states the height its elk plan settled on; a timeline states one
   *  only when it shares a row with the inspector, and otherwise flows in the
   *  column's own scroll as it always has. */
  const surfaceHeightPx = graph.graphMode
    ? graphBoxPx
    : inspectorLayout === 'side'
      ? surfaceBoxPx
      : null;

  return (
    <div className="h-full w-full bg-[#08090c] text-slate-100 flex flex-col font-sans">
      <FeatureHeader
        collapsed={headerCollapsed}
        featureId={featureId}
        featureTitle={run.featureTitle}
        status={run.status}
        statusMeta={run.statusMeta}
        currentProject={currentProject}
        remoteRun={remote.remoteRun}
        remoteMachineName={remote.remoteMachineName}
        duration={run.duration}
        totalCost={run.totalCost}
        tokens={run.tokens}
        cacheReadTokens={run.cacheReadTokens}
        cacheCreationTokens={run.cacheCreationTokens}
        stepCount={run.steps.length}
        syncing={mr.syncing}
        resolving={mr.resolving}
        publishing={mr.publishing}
        mrUrl={mr.mrUrl}
        onBack={() => navigate({ kind: 'home' })}
        onOpenTerminalTab={routing.handleOpenTerminalTab}
        onBrowseCode={routing.openEditor}
        onCancelFeature={rerun.handleCancelFeature}
        onSync={mr.handleSync}
        onPublish={mr.handlePublishClick}
        onCleanup={() => mr.handleCleanup()}
      />

      <FeatureStatusBanners
        status={run.status}
        syncBanner={mr.syncBanner}
        resolving={mr.resolving}
        onResolveConflicts={(files) => mr.handleResolveConflicts(files, null)}
        onDismissSyncBanner={() => mr.setSyncBanner(null)}
        mrUrl={mr.mrUrl}
        mrState={mr.mrState}
        onRefreshMrState={mr.refreshMrState}
      />

      {/* Above the run rather than inside it: a gate is the run asking a
          question, and it was previously findable only by scrolling to the card
          holding it (UI_REDESIGN_PLAN §3.2). */}
      <GateStrip steps={run.steps} onDecideGate={decideGate} className="mx-6 mt-4" />

      <InitialPromptPanel featureDescription={run.featureDescription} />

      <AttachmentsPanel
        attachments={attachments.attachments}
        onView={(id) => attachments.setViewingAttachmentId(id)}
      />

      {run.loading ? (
        <div className="flex-1 flex items-center justify-center">
          <RefreshCw className="w-8 h-8 text-violet-500 animate-spin" />
        </div>
      ) : run.error ? (
        <div className="flex-1 p-8 text-rose-400 flex items-center gap-2">
          <ShieldAlert className="w-5 h-5" />
          <span>Error loading details: {run.error}</span>
        </div>
      ) : (
        <div className="flex-1 flex flex-row overflow-hidden w-full h-full">
          {/* The run column. Chrome — stepper, gate table, graph — takes the
              window, however wide it is; prose panels carry their own `ch` cap.
              Artifact preview is an overlay (`ArtifactModal`) rather than a
              second track, so the two widths to negotiate are the meta track's
              and the inspector's — and they are decided by different rules on
              purpose: `pickRunLayout` answers for the meta track and cannot be
              dragged, `pickInspectorLayout` answers for the inspector and only
              until the user drags it.

              Direction is the measured width's verdict, not a breakpoint;
              `flex-row-reverse` keeps the meta panels first in the DOM — the
              order stacked reading needs — while painting them to the right of
              the run.

              `overflow-x-hidden` pairs with `overflow-y-auto` on purpose: set
              alone, `overflow-y` leaves `overflow-x` computed as `auto`, which
              is where the stray horizontal scrollbar under the timeline came
              from. */}
          <div
            ref={setRunColumnEl}
            data-run-scroll
            className={`flex w-full min-h-0 min-w-0 overflow-y-auto overflow-x-hidden p-8 ${
              runLayout === 'split' ? 'flex-row-reverse items-start gap-8' : 'flex-col'
            }`}
          >
            <RunMetaColumn
              runLayout={runLayout}
              setMetaChromeEl={setMetaChromeEl}
              remoteRun={remote.remoteRun}
              remoteMachineName={remote.remoteMachineName}
              onRunEvents={remote.handleRunEvents}
              onRemoteResolved={remote.refreshRemoteRun}
              showBootstrap={bootstrap.showBootstrap}
              bootstrapPhases={bootstrap.orderedBootstrapPhases}
              harnessBaseline={run.harnessBaseline}
              harnessEvidence={run.harnessEvidence}
            />
            <div className={runLayout === 'split' ? 'flex min-w-0 flex-1 flex-col' : 'contents'}>
              {/* One chrome row above the surface, and the element
                  `useRunColumnLayout` measures — the graph box is the column
                  minus this. The gap below it is `pb-6` rather than a margin so
                  that it lands inside `offsetHeight`; spelled as a margin it is
                  space the hook cannot see and hands to the graph twice.
                  The density control belongs to the timeline's rows, so it is
                  offered only where there are rows to compact, and the row
                  itself disappears when neither control applies rather than
                  reserving height for nothing. */}
              {(graph.canShowGraph || !graph.graphMode) && (
                <div
                  ref={setToggleChromeEl}
                  className="flex flex-wrap items-center justify-between gap-3 pb-6"
                >
                  {graph.canShowGraph && (
                    <RunViewToggle mode={graph.viewMode} onSelect={graph.setViewMode} />
                  )}
                  {!graph.graphMode && <DensityToggle value={density} onChange={setDensity} ariaLabel="Timeline density" />}
                </div>
              )}
              <RunPanes
                layout={inspectorLayout}
                surfaceHeightPx={surfaceHeightPx}
                runSurface={runSurface}
                inspector={inspector}
                inspectorWidth={inspectorWidth}
                onInspectorWidthCommit={setDraggedInspectorWidth}
              />
            </div>
          </div>
        </div>
      )}

      {/* Artifact preview. Mounted only while a path is selected — the viewer
          pulls `artifact_body` and can construct Monaco, so an always-mounted
          copy would pay that on every 3s poll tick.

          The `!view.gateStepExecutionId` guard is load-bearing, not defensive:
          the Gate overlay and this modal are both `z-50` and each binds its own
          window `keydown`, so with both mounted one Escape resolves nothing and
          dismisses both. A gate is the more urgent surface, so it wins; the
          artifact re-appears once the gate is resolved. */}
      {artifact.selectedArtifactPath && !view.gateStepExecutionId && (
        <ArtifactModal
          artifactPath={artifact.selectedArtifactPath}
          stepId={artifact.selectedStepTitle}
          contentVersion={artifact.selectedArtifactVersion}
          onClose={artifact.closeArtifact}
          onOpenEditorForPath={routing.openEditorForPath}
        />
      )}

      <AttachmentPreviewModal
        attachments={attachments.attachments}
        viewingAttachmentId={attachments.viewingAttachmentId}
        previewUrl={attachments.previewUrl}
        onClose={attachments.closePreview}
      />

      <ReplayModal
        target={rerun.replayTarget}
        status={run.status}
        overrides={overrides}
        onClose={rerun.closeReplay}
        onConfirm={rerun.handleReplayFromStep}
      />

      {/*
        The legacy `<AgentTerminalDrawer>` mount that used to live here
        was retired as part of the panel migration (spec §3 (c)). The
        "Code with Agent" button now opens a tab in the global terminal
        panel via `handleOpenTerminalTab`. Session teardown is owned by
        the panel (close button / kill-all / tray cleanup) — never by
        view unmount.
      */}
    </div>
  );
}
