import { createContext, useContext, useEffect, useReducer, useRef } from 'react';
import { createCoalescedRefresh, type CoalescedRefresh } from '../lib/coalescedRefresh';
import { getFeatureStatusRollup } from '../lib/project';
import { foldProjectActivity, type ProjectActivityMap } from '../lib/projectActivity';
import type { Project, Provider, Repository } from '../types';

export interface ProjectState {
  projects: Project[];
  currentProjectId: string | null;
  providers: Provider[];
  reposByProject: Record<string, Repository[]>;
  activityByProject: ProjectActivityMap;
  initialLoadError: string;
}

export type ProjectAction =
  | { type: 'LOAD_PROJECTS'; projects: Project[]; reposByProject: Record<string, Repository[]> }
  | { type: 'SET_CURRENT'; id: string | null }
  | { type: 'SET_PROVIDERS'; providers: Provider[] }
  | { type: 'ADD_PROJECT'; project: Project; repos?: Repository[] }
  | { type: 'UPDATE_PROJECTS'; updater: (prev: Project[]) => Project[] }
  | { type: 'REMOVE_PROJECT'; id: string }
  | { type: 'SET_PROJECT_ACTIVITY'; activity: ProjectActivityMap }
  | { type: 'SET_ERROR'; error: string };

const initial: ProjectState = {
  projects: [],
  currentProjectId: null,
  providers: [],
  reposByProject: {},
  activityByProject: {},
  initialLoadError: '',
};

export function projectReducer(state: ProjectState, action: ProjectAction): ProjectState {
  switch (action.type) {
    case 'LOAD_PROJECTS':
      return { ...state, projects: action.projects, reposByProject: action.reposByProject, initialLoadError: '' };
    case 'SET_CURRENT':
      return { ...state, currentProjectId: action.id };
    case 'SET_PROVIDERS':
      return { ...state, providers: action.providers };
    case 'ADD_PROJECT':
      return {
        ...state,
        projects: [...state.projects, action.project],
        reposByProject: action.repos
          ? { ...state.reposByProject, [action.project.id]: action.repos }
          : state.reposByProject,
      };
    case 'UPDATE_PROJECTS':
      return { ...state, projects: action.updater(state.projects) };
    case 'REMOVE_PROJECT': {
      const { [action.id]: _removed, ...rest } = state.reposByProject;
      const { [action.id]: _removedActivity, ...restActivity } = state.activityByProject;
      return {
        ...state,
        projects: state.projects.filter(p => p.id !== action.id),
        reposByProject: rest,
        activityByProject: restActivity,
      };
    }
    case 'SET_PROJECT_ACTIVITY':
      if (action.activity === state.activityByProject) return state;
      return { ...state, activityByProject: action.activity };
    case 'SET_ERROR':
      return { ...state, initialLoadError: action.error };
    default:
      return state;
  }
}

interface ProjectContextValue {
  state: ProjectState;
  dispatch: React.Dispatch<ProjectAction>;
  /**
   * Re-read the rail's per-project activity, coalesced. The live driver's
   * events trigger it from `App`, but not every write to `features.status`
   * emits one — cleanup, launch and remote reconcile each write the column
   * directly — so whoever makes such a write calls this after it lands.
   */
  refreshProjectActivity: () => void;
}

const ProjectContext = createContext<ProjectContextValue | null>(null);

export function ProjectProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(projectReducer, initial);

  // `foldProjectActivity` hands back this exact object when no count moved,
  // and the reducer bails out on that identity — so the mirror is
  // load-bearing, and is written from an effect, never during render.
  const activityRef = useRef<ProjectActivityMap>(state.activityByProject);
  useEffect(() => {
    activityRef.current = state.activityByProject;
  }, [state.activityByProject]);

  // The scheduler owns `running` / `pending` / `timer` / `lastStartedAt`,
  // so it cannot live in a `useMemo` — React may discard one at any
  // time, which would reset the cooldown and leave the discarded
  // instance's timer to fire a refresh nothing can cancel.
  const activityScheduler = useRef<CoalescedRefresh | null>(null);
  if (activityScheduler.current === null) {
    activityScheduler.current = createCoalescedRefresh(async () => {
      try {
        const rows = await getFeatureStatusRollup();
        dispatch({ type: 'SET_PROJECT_ACTIVITY', activity: foldProjectActivity(rows, activityRef.current) });
      } catch (err) {
        console.error('Failed to refresh project activity rollup:', err);
      }
    });
  }
  const refreshProjectActivity = activityScheduler.current.trigger;

  useEffect(() => () => activityScheduler.current?.dispose(), []);

  return (
    <ProjectContext.Provider value={{ state, dispatch, refreshProjectActivity }}>
      {children}
    </ProjectContext.Provider>
  );
}

export function useProject(): ProjectContextValue {
  const ctx = useContext(ProjectContext);
  if (!ctx) throw new Error('useProject must be used within ProjectProvider');
  return ctx;
}
