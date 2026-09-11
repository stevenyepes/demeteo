// Smoke tests for the ProjectRail sidebar.
//
// Spec finding C-4: the wizard entry from the project rail must sit alongside
// the existing `+` Bootstrap Project button, be labelled with a Sparkles icon,
// and route to `create-project` (the same wizard as the empty-state card's
// fourth tile) — never to the legacy `new-project` route. It must stay
// reachable when the sidebar is collapsed.

import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, type ReactElement, type ReactNode } from 'react';
import { describe, expect, it } from 'vitest';

import ProjectRail from './ProjectRail';
import {
  NavigationProvider,
  ProjectProvider,
  TerminalPanelProvider,
  UIStateProvider,
  useNavigation,
  useProject,
  useUIState,
} from '../context';
import type { ProjectActivityMap } from '../lib/projectActivity';
import type { AppView, Project } from '../types';

// Reads the active view out of NavigationContext after every render. Lives
// inside the same provider tree as ProjectRail so a click on the rail is seen.
function CaptureView({ holder }: { holder: { current: AppView | null } }): ReactElement {
  const { view } = useNavigation();
  holder.current = view;
  return <></>;
}

// Collapses the sidebar once on mount so the rail renders its collapsed branch.
function CollapseOnMount(): ReactElement {
  const { uiDispatch } = useUIState();
  useEffect(() => {
    uiDispatch({ type: 'TOGGLE_SIDEBAR' });
  }, [uiDispatch]);
  return <ProjectRail />;
}

function SeedOnMount({
  projects,
  activity,
  children,
}: {
  projects: Project[];
  activity: ProjectActivityMap;
  children: ReactNode;
}): ReactElement {
  const { dispatch } = useProject();
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects, reposByProject: {} });
    dispatch({ type: 'SET_PROJECT_ACTIVITY', activity });
  }, [dispatch, projects, activity]);
  return <>{children}</>;
}

function oneProject(name: string): Project[] {
  return [{ id: 'p1', name, status: 'idle', repos: 0, nodes: 0, spend: 0, tokens: 0 }];
}

function renderRail(children: ReactNode) {
  const holder: { current: AppView | null } = { current: null };

  render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            {children}
            <CaptureView holder={holder} />
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );

  return holder;
}

describe('ProjectRail (expanded)', () => {
  it('keeps the Bootstrap Project button alongside the new Sparkles entry', () => {
    renderRail(<ProjectRail />);

    expect(screen.getByTitle('Bootstrap Project')).toBeInTheDocument();
    expect(screen.getByTitle('New from zero')).toBeInTheDocument();
  });

  it('routes "New from zero" to create-project, not the legacy new-project route', async () => {
    const holder = renderRail(<ProjectRail />);

    await userEvent.click(screen.getByTitle('New from zero'));

    expect(holder.current?.kind).toBe('create-project');
  });
});

describe('ProjectRail (collapsed)', () => {
  it('keeps the wizard entry reachable while minimised', async () => {
    const holder = renderRail(<CollapseOnMount />);

    const sparkles = screen.getByTitle('New from zero');
    expect(sparkles).toBeInTheDocument();

    await userEvent.click(sparkles);

    expect(holder.current?.kind).toBe('create-project');
  });
});

describe('ProjectRail activity (F31)', () => {
  it('shows the attention badge and an amber dot for a project with a gate waiting', () => {
    renderRail(
      <SeedOnMount projects={oneProject('Gated')} activity={{ p1: { active: 0, needsYou: 1 } }}>
        <ProjectRail />
      </SeedOnMount>,
    );

    expect(screen.getByTestId('rail-project-attention')).toHaveTextContent('1');
    expect(screen.queryByTestId('rail-project-active')).not.toBeInTheDocument();
    expect(screen.getByTestId('status-badge')).toHaveAttribute('data-tone', 'amber');
    expect(screen.getByText('Gate needs you')).toBeInTheDocument();
  });

  it('shows the running count and a cyan dot for a project with only active runs', () => {
    renderRail(
      <SeedOnMount projects={oneProject('Busy')} activity={{ p1: { active: 2, needsYou: 0 } }}>
        <ProjectRail />
      </SeedOnMount>,
    );

    expect(screen.getByTestId('rail-project-active')).toHaveTextContent('2');
    expect(screen.queryByTestId('rail-project-attention')).not.toBeInTheDocument();
    expect(screen.getByTestId('status-badge')).toHaveAttribute('data-tone', 'cyan');
  });

  it('renders no badge at all for a project the rollup never mentioned', () => {
    renderRail(
      <SeedOnMount projects={oneProject('Quiet')} activity={{}}>
        <ProjectRail />
      </SeedOnMount>,
    );

    expect(screen.queryByTestId('rail-project-attention')).not.toBeInTheDocument();
    expect(screen.queryByTestId('rail-project-active')).not.toBeInTheDocument();
    expect(screen.getByTestId('status-badge')).toHaveAttribute('data-tone', 'emerald');
  });

  it('keeps a busy project name in the row title alongside its activity', () => {
    renderRail(
      <SeedOnMount projects={oneProject('Busy')} activity={{ p1: { active: 2, needsYou: 1 } }}>
        <ProjectRail />
      </SeedOnMount>,
    );

    expect(screen.getByTitle('Busy — 2 active · 1 needs you')).toBeInTheDocument();
  });

  it('titles a quiet project with its name alone', () => {
    renderRail(
      <SeedOnMount projects={oneProject('Quiet')} activity={{}}>
        <ProjectRail />
      </SeedOnMount>,
    );

    expect(screen.getByTitle('Quiet')).toBeInTheDocument();
  });

  it('keeps the attention badge on the initial-letter button while minimised', () => {
    renderRail(
      <SeedOnMount projects={oneProject('Gated')} activity={{ p1: { active: 0, needsYou: 1 } }}>
        <CollapseOnMount />
      </SeedOnMount>,
    );

    expect(screen.getByTestId('rail-project-attention')).toHaveTextContent('1');
  });

  it('admits the projects past the collapsed limit, and a gate waiting in one of them', () => {
    const projects: Project[] = Array.from({ length: 9 }, (_, i) => ({
      id: `p${i + 1}`, name: `Project ${i + 1}`, status: 'idle', repos: 0, nodes: 0, spend: 0, tokens: 0,
    }));
    renderRail(
      <SeedOnMount projects={projects} activity={{ p9: { active: 0, needsYou: 1 } }}>
        <CollapseOnMount />
      </SeedOnMount>,
    );

    const overflow = screen.getByTitle('1 more project — 1 needs you');
    expect(overflow).toHaveTextContent('+1');
    expect(within(overflow).getByTestId('rail-project-attention')).toHaveTextContent('1');
  });
});
