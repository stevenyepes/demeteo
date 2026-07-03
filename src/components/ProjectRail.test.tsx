// Runtime smoke tests for the ProjectRail sidebar.
//
// Spec finding C-4: the wizard entry from the project rail must sit
// alongside the existing `+` Bootstrap Project button in the header
// bar, labelled with a Sparkles icon, and route to `create-project`
// (the same wizard as the empty-state card's fourth tile).
//
// This test file verifies:
//
//   (a) The expanded header renders both the legacy Bootstrap Project
//       button AND a sibling Sparkles button titled "New from zero".
//
//   (b) Clicking the Sparkles button triggers `navigate` with
//       `{ kind: 'create-project' }` — NOT `new-project` and NOT
//       any other AppView variant.
//
//   (c) The collapsed rail also surfaces the same Sparkles button so
//       the wizard entry remains reachable while the sidebar is
//       minimised.
//
// Mirrors the runtime-throws-on-failure pattern in
// `src/wizard.renderer.test.tsx` and exports
// `projectRailTestResults` for downstream introspection.

import { act, create, type ReactTestInstance, type ReactTestRenderer } from 'react-test-renderer';
import { useEffect, type ReactElement } from 'react';

import ProjectRail from './ProjectRail';
import {
  NavigationProvider,
  ProjectProvider,
  UIStateProvider,
  useNavigation,
  useUIState,
} from '../context';
import type { AppView } from '../types';

function mount(element: ReactElement): ReactTestRenderer {
  let renderer: ReactTestRenderer | null = null;
  act(() => { renderer = create(element); });
  if (!renderer) throw new Error('ProjectRail renderer did not initialise');
  return renderer;
}

function findButtonByTitle(root: ReactTestInstance, title: string): ReactTestInstance | null {
  const buttons = root.findAll((node) => typeof node.type === 'string' && (node.type as string) === 'button');
  for (const btn of buttons) {
    if ((btn.props as { title?: string }).title === title) return btn;
  }
  return null;
}

function clickButton(btn: ReactTestInstance): void {
  const handler = (btn.props as { onClick?: () => void }).onClick;
  if (typeof handler !== 'function') {
    throw new Error(`ProjectRail: expected button to expose an onClick handler`);
  }
  act(() => { handler(); });
}

// Capture component reads the active view out of NavigationContext
// after every render. The probe lives inside the same provider tree
// as ProjectRail so the click on the rail button is observed.
function CaptureView(props: { holder: { current: AppView | null } }): ReactElement {
  const { view } = useNavigation();
  props.holder.current = view;
  return <></>;
}

// Collapse the sidebar once on mount so the rail renders the
// collapsed branch. Lives inside UIStateProvider so the rail sees the
// toggled state.
function CollapseOnMount(): ReactElement {
  const { uiDispatch } = useUIState();
  useEffect(() => { uiDispatch({ type: 'TOGGLE_SIDEBAR' }); }, [uiDispatch]);
  return <ProjectRail />;
}

// ── (a) Expanded header: Bootstrap + Sparkles buttons coexist ─────────

const expandedTree = mount(
  <NavigationProvider>
    <ProjectProvider>
      <UIStateProvider>
        <ProjectRail />
      </UIStateProvider>
    </ProjectProvider>
  </NavigationProvider>,
);

const bootstrapBtn = findButtonByTitle(expandedTree.root, 'Bootstrap Project');
if (!bootstrapBtn) {
  throw new Error('ProjectRail (expanded): Bootstrap Project button must remain in the header');
}

const newFromZeroBtn = findButtonByTitle(expandedTree.root, 'New from zero');
if (!newFromZeroBtn) {
  throw new Error('ProjectRail (expanded): "New from zero" Sparkles button missing from the header');
}

expandedTree.unmount();

// ── (b) Clicking Sparkles routes to create-project ────────────────────

const holder: { current: AppView | null } = { current: null };

const clickTree = mount(
  <NavigationProvider>
    <ProjectProvider>
      <UIStateProvider>
        <ProjectRail />
        <CaptureView holder={holder} />
      </UIStateProvider>
    </ProjectProvider>
  </NavigationProvider>,
);

const newFromZeroBtnClick = findButtonByTitle(clickTree.root, 'New from zero');
if (!newFromZeroBtnClick) {
  throw new Error('ProjectRail (expanded, second mount): "New from zero" Sparkles button missing');
}

const beforeView = holder.current;
clickButton(newFromZeroBtnClick);
const afterView = holder.current;

if (!afterView) {
  throw new Error('ProjectRail: navigation context view should be set after click');
}
if (afterView.kind !== 'create-project') {
  throw new Error(
    `ProjectRail: clicking "New from zero" must navigate to 'create-project'; got '${afterView.kind}' ` +
    `(before-click view was '${beforeView?.kind ?? 'null'}'). The legacy 'new-project' route is forbidden here.`,
  );
}

clickTree.unmount();

// ── (c) Collapsed rail also exposes the Sparkles button ───────────────

const collapsedTree = mount(
  <NavigationProvider>
    <ProjectProvider>
      <UIStateProvider>
        <CollapseOnMount />
      </UIStateProvider>
    </ProjectProvider>
  </NavigationProvider>,
);

const collapsedSparkles = findButtonByTitle(collapsedTree.root, 'New from zero');
if (!collapsedSparkles) {
  throw new Error('ProjectRail (collapsed): "New from zero" Sparkles button must also be reachable when the sidebar is collapsed');
}

// And clicking it from the collapsed branch also routes to
// create-project.
const collapsedHolder: { current: AppView | null } = { current: null };
collapsedTree.unmount();

const collapsedClickTree = mount(
  <NavigationProvider>
    <ProjectProvider>
      <UIStateProvider>
        <CollapseOnMount />
        <CaptureView holder={collapsedHolder} />
      </UIStateProvider>
    </ProjectProvider>
  </NavigationProvider>,
);

const collapsedSparklesClick = findButtonByTitle(collapsedClickTree.root, 'New from zero');
if (!collapsedSparklesClick) {
  throw new Error('ProjectRail (collapsed, second mount): "New from zero" Sparkles button missing');
}
clickButton(collapsedSparklesClick);
if (collapsedHolder.current?.kind !== 'create-project') {
  throw new Error(
    `ProjectRail (collapsed): clicking "New from zero" must navigate to 'create-project'; ` +
    `got '${collapsedHolder.current?.kind ?? 'null'}'`,
  );
}

collapsedClickTree.unmount();

export const projectRailTestResults = {
  expanded: {
    hasBootstrapButton: true,
    hasSparklesButton: true,
    sparklesRoutesTo: 'create-project',
  },
  collapsed: {
    hasSparklesButton: true,
    sparklesRoutesTo: 'create-project',
  },
} as const;