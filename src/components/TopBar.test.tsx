// Regression net for the top header bar.
//
// The AC5 cases below were written against the pre-restructure `TopBar` and
// watched green there, so they pin behaviour that already existed rather than
// whatever the three-track rewrite happened to produce. Nothing here proves the
// search is optically centred — jsdom computes no layout, and containment is
// the only honest proxy (spec §5, "Not covered by any test, by construction").

import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { type ReactElement } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import TopBar from './TopBar';
import { NavigationProvider, UIStateProvider, useNavigation, useUIState } from '../context';
import { listMirroredRuns } from '../lib/remoteRuns';
import { resizeObserverStubs } from '../test/setup';
import type { AppView, Provider, RemoteRunMirror, TerminalTabDescriptor } from '../types';

// The panel provider owns real sessions and a `Channel` round-trip to reach a
// single tab; the header only ever reads `state.tabs`, so the hook is replaced
// and the rest of the context is the real thing — navigation and UI state are
// what two of these cases assert against.
const terminalTabs: { current: TerminalTabDescriptor[] } = { current: [] };
vi.mock('../context', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../context')>()),
  useTerminalPanel: () => ({ state: { tabs: terminalTabs.current, activeTabId: null } }),
}));

vi.mock('../lib/remoteRuns', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../lib/remoteRuns')>()),
  listMirroredRuns: vi.fn(),
}));

vi.mock('../lib/notifications', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../lib/notifications')>()),
  listNotifications: vi.fn(async () => []),
  unreadNotificationCount: vi.fn(async () => 0),
}));

function mirror(status: string, id: string): RemoteRunMirror {
  return {
    machine_id: 'm1',
    run_id: id,
    project_id: null,
    title: `run ${id}`,
    status,
    error: null,
    feature_id: null,
    pr_url: null,
    pushed_branch: null,
    last_offset: 0,
    created_at: 0,
    updated_at: 0,
    last_notified_status: null,
  };
}

interface Holder {
  view: AppView | null;
  paletteOpen: boolean;
  navigate: (view: AppView) => void;
}

function Capture({ holder }: { holder: Holder }): ReactElement {
  const { view, navigate } = useNavigation();
  const { ui } = useUIState();
  holder.view = view;
  holder.paletteOpen = ui.commandPaletteOpen;
  holder.navigate = navigate;
  return <></>;
}

async function mount(connectedProvider: Provider | null = null): Promise<Holder> {
  const holder: Holder = { view: null, paletteOpen: false, navigate: () => {} };
  render(
    <NavigationProvider>
      <UIStateProvider>
        <TopBar connectedProvider={connectedProvider} />
        <Capture holder={holder} />
      </UIStateProvider>
    </NavigationProvider>,
  );
  // The mount-time `listMirroredRuns` poll resolves on a microtask and sets the
  // badge counts; without draining it here every case races its own render.
  await act(async () => {});
  return holder;
}

beforeEach(() => {
  terminalTabs.current = [];
  vi.mocked(listMirroredRuns).mockResolvedValue([]);
});

// Several components under this tree may observe; take the one registered for
// the header rather than trusting the registration order.
function observerFor(el: HTMLElement): (typeof resizeObserverStubs)[number] {
  const stub = resizeObserverStubs.find((o) => o.observe.mock.calls.some(([target]) => target === el));
  if (!stub) throw new Error('no ResizeObserver was registered for the header');
  return stub;
}

function tab(tabId: string): TerminalTabDescriptor {
  return {
    sessionId: `sess-${tabId}`,
    tabId,
    machineId: 'local',
    machineLabel: 'local',
    title: tabId,
    phase: 'running',
    createdAt: 0,
  };
}

describe('TopBar — terminals entry (AC5.1, AC5.2)', () => {
  it('navigates to the terminals view and keeps its accessible name', async () => {
    const holder = await mount();

    const toggle = screen.getByTestId('topbar-terminal-toggle');
    expect(toggle).toHaveAccessibleName('Open terminals view');

    await userEvent.click(toggle);
    expect(holder.view).toEqual({ kind: 'terminals' });
  });

  it('pulses while sessions are live and the terminals view is not open', async () => {
    terminalTabs.current = [tab('t1')];
    await mount();

    expect(screen.getByTestId('topbar-terminal-pulse')).toBeInTheDocument();
  });

  it('drops the pulse once the terminals view is the current one', async () => {
    terminalTabs.current = [tab('t1')];
    await mount();

    await userEvent.click(screen.getByTestId('topbar-terminal-toggle'));
    expect(screen.queryByTestId('topbar-terminal-pulse')).toBeNull();
  });
});

describe('TopBar — remote runs badge (AC5.3)', () => {
  it('counts the runs that need attention', async () => {
    vi.mocked(listMirroredRuns).mockResolvedValue([
      mirror('parked', 'a'),
      mirror('needs-credentials', 'b'),
      mirror('failed', 'c'),
      mirror('running', 'd'),
    ]);
    await mount();

    expect(screen.getByTestId('topbar-runs-badge')).toHaveTextContent('3');
    expect(screen.queryByTestId('topbar-runs-pulse')).toBeNull();
  });

  // The badge is a bare `<span>` and the button's `aria-label` overrides
  // contents, so the count reaches assistive tech only if the name carries it —
  // the `title` at TopBar.tsx does not, once a name is published.
  it('carries the actionable count in the accessible name', async () => {
    vi.mocked(listMirroredRuns).mockResolvedValue([
      mirror('parked', 'a'),
      mirror('needs-credentials', 'b'),
      mirror('failed', 'c'),
    ]);
    await mount();

    expect(screen.getByTestId('topbar-runs')).toHaveAccessibleName('Runs 3');
  });

  it('caps the badge at 9+', async () => {
    vi.mocked(listMirroredRuns).mockResolvedValue(
      Array.from({ length: 12 }, (_, i) => mirror('parked', `p${i}`)),
    );
    await mount();

    expect(screen.getByTestId('topbar-runs-badge')).toHaveTextContent('9+');
  });

  it('shows a running pulse and no count when only runs are in flight', async () => {
    vi.mocked(listMirroredRuns).mockResolvedValue([mirror('running', 'a'), mirror('pending', 'b')]);
    await mount();

    expect(screen.getByTestId('topbar-runs-pulse')).toBeInTheDocument();
    expect(screen.queryByTestId('topbar-runs-badge')).toBeNull();
  });

  // The base header titled the cyan dot itself; the dot is 8px of decoration and
  // `HeaderNavItem` gives it no title, so the count rides the button's title the
  // way the Terminals count already does.
  it('says how many runs are in progress when none need attention', async () => {
    vi.mocked(listMirroredRuns).mockResolvedValue([mirror('running', 'a'), mirror('running', 'b')]);
    await mount();

    expect(screen.getByTestId('topbar-runs')).toHaveAttribute(
      'title',
      'Runs — 2 remote runs in progress',
    );
  });

  it('says it in the singular for one run in progress', async () => {
    vi.mocked(listMirroredRuns).mockResolvedValue([mirror('running', 'a')]);
    await mount();

    expect(screen.getByTestId('topbar-runs')).toHaveAttribute(
      'title',
      'Runs — 1 remote run in progress',
    );
  });

  it('keeps the needs-attention title while runs are also in flight', async () => {
    vi.mocked(listMirroredRuns).mockResolvedValue([
      mirror('parked', 'a'),
      mirror('failed', 'b'),
      mirror('running', 'c'),
    ]);
    await mount();

    expect(screen.getByTestId('topbar-runs')).toHaveAttribute(
      'title',
      'Runs — 2 runs need attention',
    );
  });

  it('shows neither indicator when there are no remote runs', async () => {
    await mount();

    expect(screen.queryByTestId('topbar-runs-badge')).toBeNull();
    expect(screen.queryByTestId('topbar-runs-pulse')).toBeNull();
    expect(screen.getByTestId('topbar-runs')).toHaveAttribute(
      'title',
      'Runs — every run launched on a remote machine',
    );
  });

  it('keeps the last known counts when the poll rejects', async () => {
    vi.mocked(listMirroredRuns).mockRejectedValue(new Error('offline'));
    await mount();

    expect(screen.queryByTestId('topbar-runs-badge')).toBeNull();
    expect(screen.getByTestId('topbar-terminal-toggle')).toBeInTheDocument();
  });
});

describe('TopBar — search trigger (AC5.4)', () => {
  it('opens the command palette and still shows the shortcut hint', async () => {
    const holder = await mount();

    expect(screen.getByText('⌘K')).toBeInTheDocument();
    await userEvent.click(screen.getByText('Search workspace...'));
    expect(holder.paletteOpen).toBe(true);
  });

  // Reached by `tab()` rather than `.focus()`: the point is that the platform
  // puts the header's focal control in the tab order at all, which a
  // programmatic focus would answer for.
  it('takes keyboard focus as a control named by its own contents', async () => {
    await mount();

    const trigger = screen.getByRole('button', { name: /search workspace/i });
    expect(trigger).toBe(screen.getByTestId('topbar-search'));

    await userEvent.tab();
    expect(trigger).toHaveFocus();
  });

  it.each([
    ['{Enter}', '{Enter}'],
    ['Space', '[Space]'],
  ])('opens the command palette on %s', async (_name, keys) => {
    const holder = await mount();

    await userEvent.tab();
    expect(screen.getByTestId('topbar-search')).toHaveFocus();

    await userEvent.keyboard(keys);
    expect(holder.paletteOpen).toBe(true);
  });
});

describe('TopBar — three-track grid (AC2)', () => {
  // Containment, never class strings: jsdom computes no layout, so "the search
  // has its own track" is only observable as "the centre track owns it and
  // neither side track does".
  it('puts the search trigger in the centre track and in neither side track', async () => {
    await mount();

    const search = screen.getByTestId('topbar-search');
    expect(screen.getByTestId('topbar-center')).toContainElement(search);
    expect(screen.getByTestId('topbar-nav')).not.toContainElement(search);
    expect(screen.getByTestId('topbar-brand')).not.toContainElement(search);
  });

  it('makes the three tracks the header’s only element children, in order', async () => {
    await mount();

    const header = screen.getByTestId('topbar-nav').parentElement as HTMLElement;
    expect([...header.children].map((el) => el.getAttribute('data-testid'))).toEqual([
      'topbar-brand',
      'topbar-center',
      'topbar-nav',
    ]);
  });
});

describe('TopBar — current view in the nav', () => {
  // Accessible names, not test ids: two of the four entries have no test id, and
  // the name is the thing a screen-reader user actually pairs `aria-current`
  // with. `Runs` is its bare name only while no run needs attention — the
  // default mock resolves to none.
  const ENTRY_NAMES = ['Workflows', 'Providers', 'Runs', 'Open terminals view'];

  function currentEntries(): string[] {
    return ENTRY_NAMES.filter(
      (name) => screen.getByRole('button', { name }).getAttribute('aria-current') === 'page',
    );
  }

  it.each([
    ['Workflows', { kind: 'workflows' }],
    ['Providers', { kind: 'providers' }],
    ['Runs', { kind: 'remote-inbox' }],
    ['Open terminals view', { kind: 'terminals' }],
  ])('marks %s as the current page, and only it, once its view is open', async (name, view) => {
    const holder = await mount();
    expect(currentEntries()).toEqual([]);

    // Reached by clicking the entry: seeding the view would let the assertion
    // pass against a wiring the user cannot get to.
    await userEvent.click(screen.getByRole('button', { name }));

    expect(holder.view).toEqual(view);
    expect(currentEntries()).toEqual([name]);
  });

  // The workflow editor is its own view with its own chrome, so the header's
  // Workflows entry is not "where you are" while it is open. Pinned because the
  // obvious widening — matching a family of kinds — reads as an improvement.
  it('leaves every entry unmarked in the workflow editor', async () => {
    const holder = await mount();

    act(() => holder.navigate({ kind: 'workflow-editor', workflowId: null }));

    expect(currentEntries()).toEqual([]);
  });
});

describe('TopBar — account control (AC4)', () => {
  const provider: Provider = {
    id: 'p1',
    type: 'github',
    name: 'GitHub',
    host: 'github.com',
    pat: '',
    username: 'octocat',
    avatarUrl: 'https://example.invalid/octocat.png',
  };

  it('puts the account trigger in the nav track, named for the connected provider', async () => {
    await mount(provider);

    const trigger = screen.getByTestId('topbar-account-trigger');
    expect(screen.getByTestId('topbar-nav')).toContainElement(trigger);
    expect(trigger).toHaveAccessibleName('Account — octocat');
  });

  // AC4's other half is a *removal* — the standalone ⚙ and the bare avatar it
  // sat beside — and a removal leaves nothing to assert about, so the row is
  // pinned as an exact set instead: re-adding either one fails here rather than
  // passing unnoticed through every other case.
  it('holds exactly these controls in the nav row, in this order', async () => {
    await mount(provider);

    const nav = screen.getByTestId('topbar-nav');
    const buttons = within(nav).getAllByRole('button');
    const names = [
      'Workflows',
      'Providers',
      'Runs',
      'Open terminals view',
      'Notifications (0 unread)',
      'Account — octocat',
    ];

    expect(buttons).toHaveLength(names.length);
    names.forEach((name, i) => expect(buttons[i]).toHaveAccessibleName(name));

    // The avatar the ⚙ replaced was a bare `<img>`, which the button set above
    // cannot see come back; the trigger's own avatar is the row's only image.
    const images = [...nav.querySelectorAll('img')];
    expect(images).toHaveLength(1);
    expect(screen.getByTestId('topbar-account-trigger')).toContainElement(images[0]);
  });
});

describe('TopBar — measured density (AC3)', () => {
  function resize(header: HTMLElement, width: number): void {
    Object.defineProperty(header, 'offsetWidth', { configurable: true, value: width });
    // The stub fires with an empty entry list, so this tick only lands if the
    // observer reads the element rather than `entry.contentRect`.
    act(() => observerFor(header).trigger());
  }

  it('drops the nav labels below the icons threshold while the names survive', async () => {
    await mount();
    expect(screen.getByText('Workflows')).toBeInTheDocument();

    const header = screen.getByTestId('topbar-nav').parentElement as HTMLElement;
    resize(header, 1100);

    expect(screen.queryByText('Workflows')).toBeNull();
    expect(screen.getByRole('button', { name: /workflows/i })).toBeInTheDocument();
    expect(screen.getByTestId('topbar-terminal-toggle')).toHaveAccessibleName('Open terminals view');
  });

  // 1440 is `src-tauri/tauri.conf.json`'s default window width, and the header
  // showed these four names at every reachable width before the ladder existed.
  // Reached from `icons` rather than from the mount, so the initial density
  // cannot answer for the ladder.
  it('carries the labels at the default window width', async () => {
    await mount();
    const header = screen.getByTestId('topbar-nav').parentElement as HTMLElement;
    resize(header, 1100);
    expect(screen.queryByText('Workflows')).toBeNull();

    resize(header, 1440);

    expect(screen.getByText('Workflows')).toBeInTheDocument();
    expect(screen.getByText('Providers')).toBeInTheDocument();
    expect(screen.getByText('Runs')).toBeInTheDocument();
    expect(screen.getByText('Terminals')).toBeInTheDocument();
  });

  // The first frame renders before any observer tick, and `labels` is the side
  // of the threshold the default window lands on — pinned so a later move of
  // the constants has to revisit the seed too.
  it('renders labelled before the first resize tick', async () => {
    await mount();

    // No `resize()` above: this is the seeded density, not a measurement.
    expect(screen.getByText('Workflows')).toBeInTheDocument();
    expect(screen.getByText('Terminals')).toBeInTheDocument();
  });
});
