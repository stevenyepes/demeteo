// Smoke tests for the ProjectRail sidebar.
//
// Spec finding C-4: the wizard entry from the project rail must sit alongside
// the existing `+` Bootstrap Project button, be labelled with a Sparkles icon,
// and route to `create-project` (the same wizard as the empty-state card's
// fourth tile) — never to the legacy `new-project` route. It must stay
// reachable when the sidebar is collapsed.

import { render, screen } from '@testing-library/react';
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
  useUIState,
} from '../context';
import type { AppView } from '../types';

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
