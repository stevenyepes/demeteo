/**
 * Where the inspector sits, and who owns its width once the user has said.
 *
 * The measurement is mocked and the policy is not: `useRunColumnLayout` is the
 * one part of this that needs a laid-out DOM, and jsdom lays nothing out — its
 * `ResizeObserver` stub never reports a box, so every run column would measure
 * as unmeasured and the side-by-side branch would be unreachable from a test.
 * `pickInspectorLayout` and `defaultInspectorWidth` are already covered as pure
 * functions in `runLayout.test.ts`; what is left, and what this file is for, is
 * that the view asks them and then honours the answer.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, useState, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

let columnWidth = 1600;
let runLayoutMode: 'stacked' | 'split' = 'stacked';
/** The element the view offers the hook as "the chrome above the surface". */
let toggleChromeEl: HTMLElement | null = null;
/** The `app_session` rows this mount finds already written. */
let stored: Record<string, string> = {};

vi.mock('../useRunColumnLayout', () => ({
  useRunColumnLayout: () => ({
    setRunColumnEl: () => {},
    runColumnEl: null,
    setMetaChromeEl: () => {},
    setToggleChromeEl: (el: HTMLElement | null) => {
      toggleChromeEl = el;
    },
    runColumnSize: { width: columnWidth, height: 900 },
    runLayout: runLayoutMode,
    graphBoxPx: 448,
  }),
}));

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}));

import {
  NavigationProvider,
  ProjectProvider,
  TerminalPanelProvider,
  UIStateProvider,
  useNavigation,
} from '../../context';
import { inspectorWidthPref } from '../../lib/uiPrefs';
import type { StepExecution } from '../../types';
import { defaultInspectorWidth } from '../runLayout';
import { FeatureDetail } from './FeatureDetail';

const FEATURE_ID = 'f-1';

const STEP: StepExecution = {
  id: 'se-1',
  feature_id: FEATURE_ID,
  step_id: 's-research',
  step_index: 0,
  step_kind: 'agent',
  status: 'completed',
  artifact_paths: [],
  created_at: 0,
  updated_at: 0,
};

function mockBackend() {
  vi.mocked(invoke).mockImplementation(((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'step_list_for_run':
        return Promise.resolve([STEP]);
      case 'sync_session_get':
        return Promise.resolve(null);
      case 'feature_get':
        return Promise.resolve({ id: FEATURE_ID, status: 'completed' });
      case 'feature_workflow_graph':
        return Promise.resolve(null);
      case 'get_app_session':
        return Promise.resolve(stored[String(args?.key)] ?? null);
      case 'set_app_session':
        return Promise.resolve(undefined);
      case 'feature_list_attachments':
      case 'get_machines':
      case 'list_agents':
      case 'list_terminal_sessions':
      case 'step_attempts_list':
        return Promise.resolve([]);
      case 'remote_run_for_feature':
        return Promise.resolve(null);
      default:
        return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
    }
  }) as unknown as typeof invoke);
}

/** Re-renders the tree when `bump` changes, which is how a column resize
 *  reaches the view through the mocked hook. */
function Seed({ onResize }: { onResize?: (resize: () => void) => void }) {
  const { navigate } = useNavigation();
  const [, setBump] = useState(0);
  useEffect(() => {
    navigate({ kind: 'detail', featureId: FEATURE_ID, featureTitle: 'Run' });
    onResize?.(() => setBump((n) => n + 1));
  }, [navigate, onResize]);
  return <FeatureDetail />;
}

function mount(onResize?: (resize: () => void) => void) {
  return render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <Seed onResize={onResize} />
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

beforeEach(() => {
  columnWidth = 1600;
  runLayoutMode = 'stacked';
  toggleChromeEl = null;
  stored = {};
  mockBackend();
});

afterEach(() => {
  vi.restoreAllMocks();
});

/**
 * Inherited from `RunViewToggle.test.tsx`, where it was an assertion about a
 * `chromeRef` prop that no longer exists: the chrome above the run surface is a
 * row holding the view toggle *and* the density toggle now, so the row is what
 * has to be measured. The claim is unchanged — chrome the hook cannot see is
 * height the graph box claims twice.
 */
describe('the chrome the graph box is measured against', () => {
  it('offers the hook the whole row, spacing included', async () => {
    mount();

    // This feature has no graph definition, so the view toggle is absent and the
    // density control is the only occupant — the case that previously had no
    // chrome row at all and so had nothing to measure.
    const density = await screen.findByRole('radiogroup', { name: 'Timeline density' });
    await waitFor(() => expect(toggleChromeEl).not.toBeNull());
    expect(toggleChromeEl).toContainElement(density);

    // `offsetHeight` excludes margin. The gap under this row must therefore be
    // padding, or the hook reports a row shorter than the space it occupies.
    expect(toggleChromeEl?.className).toMatch(/\bpb-\d/);
    expect(toggleChromeEl?.className).not.toMatch(/\bmb-\d/);
  });
});

describe('where the run\u2019s three tracks begin', () => {
  it('starts the meta track on the same line as the panes beside it', async () => {
    // The chrome row used to sit *inside* the graph\u2019s track, so it pushed two
    // of the three tracks down and left the meta panels beginning a row higher
    // than the cards they stand next to \u2014 three peers with one of them floating.
    // jsdom lays nothing out, so the claim is structural: nothing between the
    // chrome and the tracks, and both tracks under one parent that starts below
    // it.
    runLayoutMode = 'split';
    mount();
    await waitFor(() => expect(toggleChromeEl).not.toBeNull());

    const meta = screen.getByTestId('run-meta-column');
    const panes = await screen.findByTestId('split-pane');
    const tracks = toggleChromeEl?.nextElementSibling ?? null;

    expect(tracks).not.toBeNull();
    expect(tracks).toContainElement(meta);
    expect(tracks).toContainElement(panes);
    expect(meta).not.toContainElement(toggleChromeEl);
    expect(panes).not.toContainElement(toggleChromeEl);
  });
});

describe('the inspector\u2019s place in the run column', () => {
  it('sits beside the run surface in a column wide enough for both', async () => {
    mount();

    const inspector = await screen.findByTestId('inspector');
    expect(screen.getByTestId('split-pane-secondary')).toContainElement(inspector);
    // The run surface is the primary pane, not a sibling of the split.
    expect(screen.getByTestId('split-pane')).toContainElement(
      screen.getByRole('list', { name: 'Run steps' }),
    );
  });

  it('drops below it in a column that cannot seat two panes', async () => {
    columnWidth = 700;
    mount();

    expect(await screen.findByTestId('inspector')).toBeInTheDocument();
    // Never hidden, never behind an affordance — only moved (§7).
    expect(screen.queryByTestId('split-pane')).not.toBeInTheDocument();
  });

  it('opens at a proportion of the column and hands it over on the first drag', async () => {
    // The divider refuses every key while its container is unmeasured, and
    // jsdom measures nothing — so the box has to be given a width by hand for
    // the keyboard path to exist at all.
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
      this: HTMLElement,
    ) {
      const width = this.dataset.testid === 'split-pane' ? 1200 : 0;
      return { width, height: 0, top: 0, left: 0, right: width, bottom: 0, x: 0, y: 0 } as DOMRect;
    });

    let resize = () => {};
    mount((fn) => {
      resize = fn;
    });

    const handle = await screen.findByTestId('split-pane-handle');
    const opening = handle.getAttribute('aria-valuenow');
    expect(Number(opening)).toBeGreaterThanOrEqual(320);

    handle.focus();
    // `Home`, not `End`: the pane opens at half the row it is given, and this
    // box is stubbed narrow enough that half of it *is* the divider's ceiling —
    // so `End` asks for the width the pane already has and the assertion below
    // passes or fails on whether the observer happened to fire first.
    await userEvent.keyboard('{Home}');
    const chosen = handle.getAttribute('aria-valuenow');
    expect(chosen).not.toBe(opening);

    // A width the user chose outranks every number the layout module produces,
    // so a re-measure may not quietly reset it (`runLayout.ts`: ask once).
    columnWidth = 1900;
    await waitFor(() => resize());
    expect(handle.getAttribute('aria-valuenow')).toBe(chosen);
  });

  it('opens at a stored width, which outranks the column exactly as a drag would', async () => {
    stored[inspectorWidthPref.key] = '640';
    let resize = () => {};
    mount((fn) => {
      resize = fn;
    });

    const handle = await screen.findByTestId('split-pane-handle');
    await waitFor(() => expect(handle.getAttribute('aria-valuenow')).toBe('640'));
    // A restored width that merely *looked* right would pass the line above by
    // coincidence if it were the proportion this column derives anyway.
    expect(defaultInspectorWidth({ width: columnWidth, height: 900 })).not.toBe(640);

    // The half of the claim a restore could plausibly get wrong: arriving as
    // state rather than through a drag, it must still stop the column from
    // re-deriving a width — a restore is a choice, not an opening default.
    columnWidth = 1900;
    await waitFor(() => resize());
    expect(handle.getAttribute('aria-valuenow')).toBe('640');
  });
});
