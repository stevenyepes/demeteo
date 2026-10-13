import { useEffect, useRef } from 'react';
import { useNavigation, useProject } from '../context';
import { getFeature } from '../lib/featureSync';

/**
 * Keep the selected workspace on the project that owns the feature the detail
 * view is showing.
 *
 * Several routes open a feature without knowing — or caring — which project is
 * selected: the `gate_required` overlay, the Runs inbox, an Ask canvas node,
 * Back/Forward through history. Each used to leave `currentProjectId` where it
 * was, so a gate from another project opened over a rail still highlighting the
 * old one, and everything scoped to the current project (Cmd+G cycling, the
 * Ask view, launch) silently aimed at the wrong place. Following the view here
 * covers every route at once, including ones added later.
 *
 * Keyed on the feature alone: switching project from the rail always navigates
 * home, so there is no detail view for this to fight.
 */
export function useFollowFeatureProject(): void {
  const { view } = useNavigation();
  const { state: { currentProjectId, projects }, dispatch } = useProject();
  const featureId = view.kind === 'detail' ? view.featureId : null;

  const currentRef = useRef(currentProjectId);
  currentRef.current = currentProjectId;
  const projectsRef = useRef(projects);
  projectsRef.current = projects;

  useEffect(() => {
    if (featureId === null) return;
    let cancelled = false;
    getFeature(featureId).then(
      (feature) => {
        if (cancelled || !feature?.project_id) return;
        if (feature.project_id === currentRef.current) return;
        // Selecting an id the rail does not hold would leave no current
        // project at all, which is worse than the wrong one.
        if (!projectsRef.current.some((p) => p.id === feature.project_id)) return;
        dispatch({ type: 'SET_CURRENT', id: feature.project_id });
      },
      (err) => {
        console.error('Failed to resolve the project of feature', featureId, err);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [featureId, dispatch]);
}
