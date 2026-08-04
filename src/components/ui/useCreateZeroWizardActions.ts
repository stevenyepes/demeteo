import { useCallback, useMemo, useState } from 'react';
import { useNavigation } from '../../context';
import { formatError } from '../../lib/errors';
import { useErrorBus } from '../../lib/errorBus';
import { saveProjectSettings } from '../../lib/project';
import { startFeature } from '../../lib/createProjectWizard';
import type { WizardFormApi } from './useCreateZeroWizardForm';

const WORKFLOW_ID_STARTER = 'wf-starter-standard';

export interface LaunchState {
  launching: boolean;
  errorMessage: string | null;
  retry: () => Promise<void>;
}

/**
 * Side-effectful orchestration helpers for the Create-From-Zero
 * wizard. The bootstrap pipeline lives in `useCreateZeroBootstrap`;
 * this hook owns the strategy-approval save + the start_feature
 * launch path. The wizard owns the step state machine.
 */
export function useCreateZeroWizardActions(form: WizardFormApi) {
  const { navigate } = useNavigation();
  const { reportError } = useErrorBus();
  const [launching, setLaunching] = useState(false);
  const [launchError, setLaunchError] = useState<string | null>(null);

  // Step 6 → 7: approve the detected strategy and move on.
  const approveStrategy = useCallback(async () => {
    if (!form.projectId) return;
    try {
      await saveProjectSettings(form.projectId, {
        default_branch: form.defaultBranch,
        branch_prefix: form.branchPrefix,
        test_command: form.testCommand.trim() || null,
        pr_template: form.prTemplate || null,
        conflict_policy: form.conflictPolicy,
        feature_lifecycle: form.featureLifecycle,
      });
    } catch (err) { reportError(err, { kind: 'internal' }); }
  }, [form, reportError]);

  // Step 9: launch the feature via the same wrapper the modal uses,
  // then navigate to the detail view. On failure, exposes the error
  // on `launchState.errorMessage` so the LaunchStep component can
  // render an inline retry CTA.
  const launchFeature = useCallback(async () => {
    if (!form.projectId) return;
    setLaunching(true);
    setLaunchError(null);
    try {
      const title = form.projectName.trim() || 'New feature';
      const feature = await startFeature({
        projectId: form.projectId,
        workflowId: form.workflowId || WORKFLOW_ID_STARTER,
        title,
        description: form.description.trim(),
        agentKind: form.agentKind || null,
        model: form.model || null,
        effort: form.effort || null,
        stagedAttachments: [],
      });
      navigate({ kind: 'detail', featureId: feature.id, featureTitle: feature.title ?? title });
    } catch (err) {
      setLaunching(false);
      setLaunchError(formatError(err));
    }
  }, [form, navigate]);

  const launchState: LaunchState = useMemo(() => ({
    launching,
    errorMessage: launchError,
    retry: launchFeature,
  }), [launching, launchError, launchFeature]);

  return { approveStrategy, launchFeature, launchState };
}
