import { useMemo } from 'react';

import { useNavigation, useProject, useUIState } from '../context';
import type { LaunchRunOptions, LaunchRunParams } from '../hooks/useLaunchRun';
import type { Feature } from '../types';
import StartFeatureModal from './StartFeatureModal';

interface StartFeatureHostProps {
  launchRun: (params: LaunchRunParams, options?: LaunchRunOptions) => Promise<Feature | null>;
}

/**
 * The app-level Start Feature modal, opened and closed through UI state.
 *
 * It is portalled over every view, so nothing unmounts it when the route
 * changes: a control that leaves it must close it explicitly, or the user
 * lands on a page they cannot see. There is one such exit, the notice's
 * settings link; the refusal toast in `useLaunchRun` reaches it through
 * `LEAVE_START_FEATURE`, so the kept draft is always the modal's own. This is
 * deliberately not driven by view changes: the inspector seeding a step on the
 * detail view underneath, or a gate a background run raises, replaces the view
 * without the user asking to go anywhere, and closing on those would discard
 * the draft mid-typing.
 */
export function StartFeatureHost({ launchRun }: StartFeatureHostProps) {
  const { navigate } = useNavigation();
  const { state: proj } = useProject();
  const { ui, uiDispatch } = useUIState();
  const { projects, currentProjectId, reposByProject } = proj;
  const { startFeatureOpen, startFeatureWorkflowId, startFeatureSeed, startFeatureLeaveRequested } = ui;
  const currentProject = useMemo(
    () => projects.find((p) => p.id === currentProjectId) ?? null,
    [projects, currentProjectId],
  );

  if (!startFeatureOpen || !currentProjectId || !currentProject) return null;

  const seedIsForeign = !!startFeatureSeed?.projectId && startFeatureSeed.projectId !== currentProjectId;
  // A close without `keepDraft` clears the kept seed, so one that belongs to
  // another project is handed back rather than discarded by this project's close.
  const close = () =>
    uiDispatch({ type: 'CLOSE_START_FEATURE', keepDraft: seedIsForeign ? startFeatureSeed : undefined });

  return (
    <StartFeatureModal
      isOpen={startFeatureOpen}
      projectId={currentProjectId}
      projectName={currentProject.name}
      computeType={currentProject.compute_type}
      remoteHost={currentProject.remote_host}
      repositories={reposByProject[currentProjectId] || []}
      defaultWorkflowId={startFeatureWorkflowId}
      seed={seedIsForeign ? null : startFeatureSeed}
      onClose={close}
      leaveRequested={startFeatureLeaveRequested}
      onOpenMachineSettings={(draft) => {
        uiDispatch({ type: 'CLOSE_START_FEATURE', keepDraft: draft });
        navigate({ kind: 'settings' });
      }}
      onLaunch={async (params) => {
        let refused = false;
        const feature = await launchRun(params, {
          onRunnerRefused: () => {
            refused = true;
          },
        });
        if (feature) {
          close();
          return 'launched';
        }
        return refused ? 'runner_refused' : 'failed';
      }}
    />
  );
}
