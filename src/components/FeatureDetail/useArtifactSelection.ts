import { useCallback, useState } from 'react';
import type { StepExecution } from '../../types';

/** The artifact the preview modal is showing, and the step it belongs to. */
export function useArtifactSelection(steps: StepExecution[]) {
  const [selectedArtifactPath, setSelectedArtifactPath] = useState<string | null>(null);
  const [selectedStepTitle, setSelectedStepTitle] = useState<string | null>(null);

  const openArtifact = useCallback((path: string, stepTitle: string | null) => {
    setSelectedArtifactPath(path);
    setSelectedStepTitle(stepTitle);
  }, []);

  const closeArtifact = useCallback(() => {
    setSelectedArtifactPath(null);
    setSelectedStepTitle(null);
  }, []);

  // A shadow step's artifact file can be overwritten in place at the same path
  // by a forced re-pull (see `cache_step_artifacts` on the backend) — key on the
  // same fields that signal a fresh attempt completed so the open viewer
  // re-fetches instead of keeping the pre-overwrite content on screen. Derived
  // as a plain string, never an object or a fresh closure: `ArtifactViewer` is
  // memoized and this view polls every 3s.
  const selectedArtifactStep = steps.find((s) => s.step_id === selectedStepTitle);
  const selectedArtifactVersion = selectedArtifactStep
    ? `${selectedArtifactStep.status}:${selectedArtifactStep.tokens}:${selectedArtifactStep.wall_clock_secs}:${selectedArtifactStep.cost_usd}`
    : undefined;

  return {
    selectedArtifactPath,
    selectedStepTitle,
    selectedArtifactVersion,
    openArtifact,
    closeArtifact,
  };
}
