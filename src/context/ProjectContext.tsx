import { createContext, useContext, useReducer } from 'react';
import type { Project, Provider, Repository, WorkflowSummary } from '../types';

export interface ProjectState {
  projects: Project[];
  currentProjectId: string | null;
  providers: Provider[];
  reposByProject: Record<string, Repository[]>;
  workflowsForModal: WorkflowSummary[];
  initialLoadError: string;
}

export type ProjectAction =
  | { type: 'LOAD_PROJECTS'; projects: Project[]; reposByProject: Record<string, Repository[]> }
  | { type: 'SET_CURRENT'; id: string | null }
  | { type: 'SET_PROVIDERS'; providers: Provider[] }
  | { type: 'ADD_PROJECT'; project: Project; repos?: Repository[] }
  | { type: 'UPDATE_PROJECTS'; updater: (prev: Project[]) => Project[] }
  | { type: 'REMOVE_PROJECT'; id: string }
  | { type: 'SET_WORKFLOWS_FOR_MODAL'; workflows: WorkflowSummary[] }
  | { type: 'SET_ERROR'; error: string };

const initial: ProjectState = {
  projects: [],
  currentProjectId: null,
  providers: [],
  reposByProject: {},
  workflowsForModal: [],
  initialLoadError: '',
};

function reducer(state: ProjectState, action: ProjectAction): ProjectState {
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
      return { ...state, projects: state.projects.filter(p => p.id !== action.id), reposByProject: rest };
    }
    case 'SET_WORKFLOWS_FOR_MODAL':
      return { ...state, workflowsForModal: action.workflows };
    case 'SET_ERROR':
      return { ...state, initialLoadError: action.error };
    default:
      return state;
  }
}

interface ProjectContextValue {
  state: ProjectState;
  dispatch: React.Dispatch<ProjectAction>;
}

const ProjectContext = createContext<ProjectContextValue | null>(null);

export function ProjectProvider({ children }: { children: React.ReactNode }) {
  const [state, dispatch] = useReducer(reducer, initial);
  return (
    <ProjectContext.Provider value={{ state, dispatch }}>
      {children}
    </ProjectContext.Provider>
  );
}

export function useProject(): ProjectContextValue {
  const ctx = useContext(ProjectContext);
  if (!ctx) throw new Error('useProject must be used within ProjectProvider');
  return ctx;
}
