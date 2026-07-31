import { RefreshCw, ShieldAlert } from 'lucide-react';
import { useTauriEvent } from '../../hooks/useTauriEvent';
import { useNavigation, useProject, useUIState } from '../../context';
import { ArtifactModal } from '../ArtifactModal';
import { RunViewToggle } from '../RunViewToggle';
import { useRunColumnLayout } from '../useRunColumnLayout';
import { AttachmentPreviewModal } from './AttachmentPreviewModal';
import { AttachmentsPanel } from './AttachmentsPanel';
import { FeatureHeader } from './FeatureHeader';
import { FeatureStatusBanners } from './FeatureStatusBanners';
import { InitialPromptPanel } from './InitialPromptPanel';
import { ReplayModal } from './ReplayModal';
import { RunGraphPanel } from './RunGraphPanel';
import { RunMetaColumn } from './RunMetaColumn';
import { StepTimeline } from './StepTimeline';
import { useAgentStream } from './useAgentStream';
import { useArtifactSelection } from './useArtifactSelection';
import { useAttachmentPreview } from './useAttachmentPreview';
import { useBootstrapPhases } from './useBootstrapPhases';
import { useFeatureMr } from './useFeatureMr';
import { useFeatureRun } from './useFeatureRun';
import { useGateCardScroll } from './useGateCardScroll';
import { useHarnessOverrides } from './useHarnessOverrides';
import { useRemoteRun } from './useRemoteRun';
import { useRerunActions } from './useRerunActions';
import { useRunGraph } from './useRunGraph';
import { useWorktreeRouting } from './useWorktreeRouting';

export function FeatureDetail() {
  const { view, navigate } = useNavigation();
  const { state: { currentProjectId, projects } } = useProject();
  const { ui: { sidebarCollapsed: _sidebarCollapsed } } = useUIState();
  // The legacy `AgentTerminalDrawer` mount consumed `sidebarCollapsed`
  // to size its drawer; the panel-mounted surface does not need it.
  // We keep the field on `UIStateSlice` for back-compat with the
  // `pickEscapeAction` helper exported from `App.tsx`.
  void _sidebarCollapsed;

  if (view.kind !== 'detail') return null;
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
  const graph = useRunGraph({
    featureId,
    featureTitle: run.featureTitle,
    steps: run.steps,
    navigate,
    startReplay: rerun.startReplay,
    openArtifact: artifact.openArtifact,
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
  const { setRunColumnEl, setMetaChromeEl, setToggleChromeEl, runLayout, graphBoxPx } =
    useRunColumnLayout(graph.graphDef);

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

  const decideGate = (stepExecutionId: string) =>
    navigate({ kind: 'detail', featureId, featureTitle: run.featureTitle, gateStepExecutionId: stepExecutionId });

  // The unified feed the node panel reads: local runs push it through
  // `useRunEvents`; remote runs fill `remoteRunEvents` from the poll above.
  const panelRunEvents = remote.remoteRun ? remote.remoteRunEvents : graph.localRunEvents;

  const { graphDef, selectedNode, selectedRun, selectedStep } = graph;
  const gateStepId =
    selectedNode?.type === 'gate' ? selectedRun?.stepExecutionId ?? null : null;

  return (
    <div className="h-full w-full bg-[#08090c] text-slate-100 flex flex-col font-sans">
      <FeatureHeader
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
              second track, so the only width to negotiate is the meta track's.
              Direction is `pickRunLayout`'s verdict on the measured width, not
              a breakpoint; `flex-row-reverse` keeps the meta panels first in
              the DOM — the order stacked reading needs — while painting them
              to the right of the run.

              `overflow-x-hidden` pairs with `overflow-y-auto` on purpose: set
              alone, `overflow-y` leaves `overflow-x` computed as `auto`, which
              is where the stray horizontal scrollbar under the timeline came
              from. */}
          <div
            ref={setRunColumnEl}
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
              {graph.canShowGraph && (
                <RunViewToggle mode={graph.viewMode} onSelect={graph.setViewMode} chromeRef={setToggleChromeEl} />
              )}
              {graph.graphMode && graphDef ? (
                <RunGraphPanel
                  featureId={featureId}
                  definition={graphDef}
                  statusByNode={graph.runStatusByNode}
                  highlightedNodeIds={rerun.replayPreviewNodes}
                  graphBoxPx={graphBoxPx}
                  selectedNodeId={graph.selectedNodeId}
                  selectedNode={selectedNode}
                  selectedRun={selectedRun}
                  selectedStep={selectedStep}
                  selectedBlockedBy={graph.selectedBlockedBy}
                  runEvents={panelRunEvents}
                  liveStream={selectedStep ? stream.streamContent[selectedStep.id] : undefined}
                  onNodeActivate={graph.onNodeActivate}
                  onCloseNode={() => graph.setSelectedNodeId(null)}
                  onOpenEditorForPath={routing.openEditorForPath}
                  onOpenArtifact={graph.openArtifactFromPanel}
                  onRetry={
                    selectedStep &&
                    (selectedStep.status === 'failed' || selectedStep.status === 'interrupted')
                      ? () => rerun.handleRetryStep(selectedStep.id)
                      : undefined
                  }
                  onReplay={selectedStep ? graph.startReplayFromPanel : undefined}
                  onStop={
                    selectedStep?.status === 'running' || selectedStep?.status === 'verifying'
                      ? rerun.handleStopStep
                      : undefined
                  }
                  onDecideGate={gateStepId ? () => decideGate(gateStepId) : undefined}
                />
              ) : (
                <StepTimeline
                  steps={run.steps}
                  remoteRun={remote.remoteRun}
                  remoteMachineName={remote.remoteMachineName}
                  hasBootstrapPhases={bootstrap.bootstrapPhases.size > 0}
                  gateStepExecutionId={view.gateStepExecutionId}
                  stepCardRefs={stepCardRefs}
                  harnessBaseline={run.harnessBaseline}
                  overrides={overrides}
                  selectedArtifactPath={artifact.selectedArtifactPath}
                  activeStreamId={stream.activeStreamId}
                  streamContent={stream.streamContent}
                  onToggleStream={(id) => stream.setActiveStreamId(stream.activeStreamId === id ? null : id)}
                  onOpenArtifact={artifact.openArtifact}
                  onStartReplay={(target) => rerun.startReplay(target, null)}
                  onRetry={rerun.handleRetryStep}
                  onStop={rerun.handleStopStep}
                  onDecideGate={decideGate}
                />
              )}
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
