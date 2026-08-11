import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, vi } from "vitest";

// Setup files run once per test file, so mocks registered here apply to every
// suite without each one re-declaring the same Tauri/browser scaffolding.

afterEach(async () => {
  cleanup();
  // A `UiPref` is a module singleton, so its debounced write outlives the
  // component that armed it *and* the test that mounted it: a suite that
  // toggles a persisted control under real timers had its `set_app_session`
  // land 400 ms later inside an unrelated test, where `clearAllMocks` below
  // made it read as that test's own write. Draining here keeps the leak
  // inside the test that caused it.
  //
  // Imported here rather than at the top of the file: `uiPrefs` imports the
  // Tauri core module this file mocks, and a static import of it runs before
  // the hoisted `vi.mock` factory's own dependencies exist.
  const { UI_PREFS } = await import("../lib/uiPrefs");
  for (const pref of UI_PREFS) pref.cancelPendingWrite();
  vi.clearAllMocks();
  webglAddonStubs.length = 0;
  terminalStubs.length = 0;
  fitAddonStubs.length = 0;
  resizeObserverStubs.length = 0;
  setFitGeometry(DEFAULT_FIT_COLS, DEFAULT_FIT_ROWS);
});

// --- Tauri IPC -------------------------------------------------------------
// Every component that touches the backend goes through `invoke`. Outside a
// Tauri host there is no IPC bridge, so an unmocked call throws during render
// and the failure surfaces as an unrelated "cannot read property of undefined".
// Default to a resolved no-op; a test that cares about a specific command
// overrides it with `vi.mocked(invoke).mockResolvedValue(...)`.
//
// The terminal-panel wrappers added in spec §2.1 / §4 have callers that
// immediately iterate or property-access the return value (e.g. the panel
// host does `state.tabs.map(...)` after `listTerminalSessions()`), so a
// bare `undefined` blows up jsdom tests with a confusing TypeError. We
// dispatch on the command name and return a sensible empty payload for
// the list-shaped commands; the three void commands keep the previous
// `undefined` resolution.
// Tauri `Channel` is a class whose constructor reaches into
// `window.__TAURI_INTERNALS__.transformCallback` — that bridge is missing
// under jsdom, so a real `new Channel()` throws on instantiation. The panel
// surfaces (`src/components/TerminalSurface.tsx`) and the panel hook
// (`src/context/TerminalPanelProvider.tsx`) both instantiate one on every
// open/attach round-trip, so the test setup replaces the constructor with
// a spy-capable stub that satisfies the `Channel<T>` surface (just `.onmessage`
// and `.send` / `.close` are ever touched — everything else is plumbing).
class ChannelStub<T = unknown> {
  id = Math.floor(Math.random() * 1e9);
  onmessage: ((message: T) => void) | null = null;
  send = vi.fn().mockResolvedValue(undefined);
  close = vi.fn();
}

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_terminal_sessions":
        return Promise.resolve([]);
      case "attach_terminal_session":
      case "detach_terminal_session":
      case "rename_terminal_session":
        return Promise.resolve(undefined);
      default:
        return Promise.resolve(undefined);
    }
  }),
  convertFileSrc: vi.fn((path: string) => path),
  Channel: ChannelStub,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
  emit: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
  save: vi.fn().mockResolvedValue(null),
  message: vi.fn().mockResolvedValue(undefined),
  confirm: vi.fn().mockResolvedValue(true),
}));

// --- Heavy editors ---------------------------------------------------------
// Monaco and xterm drive real canvas/WebGL surfaces that jsdom does not
// implement. Rendering them for a component test buys nothing and costs
// seconds, so both are replaced by inert stand-ins that still let assertions
// see "the editor is here" and read its value.
const MonacoStub = ({
  value,
  onChange,
}: {
  value?: string;
  onChange?: (v?: string) => void;
}) =>
  createElement("textarea", {
    "data-testid": "monaco-editor",
    value: value ?? "",
    onChange: (e: { target: { value: string } }) => onChange?.(e.target.value),
  });

vi.mock("@monaco-editor/react", () => ({
  default: MonacoStub,
  Editor: MonacoStub,
}));

// --- xterm.js ------------------------------------------------------------
//
// The global terminal panel (src/components/TerminalSurface.tsx) owns
// an `@xterm/xterm` `Terminal` plus a `FitAddon`. Real xterm reaches
// for canvas/WebGL surfaces and ResizeObserver geometries that jsdom
// does not implement, and rendering the panel host in a component test
// would otherwise blow up trying to `term.open(div)`.
//
// We replace both with inert stand-ins: a no-op `Terminal` whose public
// methods (`write`, `clear`, `dispose`, `onData`, …) are all spy-able
// jest functions so tests can still assert "the surface called attach"
// without actually painting anything.

export const terminalStubs: TerminalStub[] = [];

/** xterm's own defaults, and the geometry every fit lands on unless a test
 *  asks for another one. */
export const DEFAULT_FIT_COLS = 80;
export const DEFAULT_FIT_ROWS = 24;

// The size a `fit()` resolves to is the one thing a terminal test needs to
// control and the one thing a real `FitAddon` derives from a layout box jsdom
// never computes. A `fit()` that measures nothing and reports 80x24 forever
// answers identically whether or not the surface has a box — so a fit taken
// inside a `display:none` subtree is indistinguishable from a correct one, and
// no test written against these doubles can observe it. Geometry is therefore
// injected, and defaults to xterm's own size so it stays opt-in per test.
export const fitGeometry = { cols: DEFAULT_FIT_COLS, rows: DEFAULT_FIT_ROWS };

/** Geometry the next `FitAddonStub.fit()` writes into its `Terminal`. */
export function setFitGeometry(cols: number, rows: number): void {
  fitGeometry.cols = cols;
  fitGeometry.rows = rows;
}

export class TerminalStub {
  write = vi.fn();
  clear = vi.fn();
  dispose = vi.fn();
  reset = vi.fn();
  refresh = vi.fn();
  onData = vi.fn();
  // Real xterm hands the addon an `ITerminalAddon` activation context carrying
  // the terminal; the fit double needs the same association to have something
  // to resize.
  loadAddon = vi.fn((addon: unknown) => {
    if (addon instanceof FitAddonStub) addon.terminal = this;
  });
  open = vi.fn();
  cols = DEFAULT_FIT_COLS;
  rows = DEFAULT_FIT_ROWS;

  constructor() {
    terminalStubs.push(this);
  }
}

export const fitAddonStubs: FitAddonStub[] = [];

export class FitAddonStub {
  terminal: TerminalStub | null = null;
  fit = vi.fn(() => {
    if (!this.terminal) return;
    this.terminal.cols = fitGeometry.cols;
    this.terminal.rows = fitGeometry.rows;
  });
  proposeDimensions = vi.fn(() => ({ cols: fitGeometry.cols, rows: fitGeometry.rows }));

  constructor() {
    fitAddonStubs.push(this);
  }
}

// WebglAddon reaches for a real WebGL context, which jsdom does not provide.
// Inert stand-in: `onContextLoss` captures the registered callback (so a test
// can simulate a GPU context loss) and `dispose` is a spy, so
// TerminalSurface's renderer-setup path runs without touching the GPU.
export const webglAddonStubs: WebglAddonStub[] = [];

class WebglAddonStub {
  contextLossHandler: (() => void) | null = null;
  onContextLoss = vi.fn((cb: () => void) => {
    this.contextLossHandler = cb;
  });
  dispose = vi.fn();

  constructor() {
    webglAddonStubs.push(this);
  }
}

vi.mock("@xterm/xterm", () => ({
  Terminal: TerminalStub,
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: FitAddonStub,
}));

vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: WebglAddonStub,
}));

// --- Browser APIs jsdom omits ----------------------------------------------
// Node 22+ can install an experimental `localStorage` accessor. In harness
// workers it may be unavailable without `--localstorage-file`, and it shadows
// jsdom's storage on both `globalThis` and `window`. Tests need browser-shaped,
// per-worker storage, so provide it without reading the host accessor at all.
const testStorage = new Map<string, string>();
const localStorageStub: Storage = {
  get length() {
    return testStorage.size;
  },
  clear() {
    testStorage.clear();
  },
  getItem(key) {
    return testStorage.get(key) ?? null;
  },
  key(index) {
    return [...testStorage.keys()][index] ?? null;
  },
  removeItem(key) {
    testStorage.delete(key);
  },
  setItem(key, value) {
    testStorage.set(key, value);
  },
};

Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: localStorageStub,
});

// jsdom lays nothing out, so no observer here ever fires on its own: a resize
// tick has to be driven by hand, which is only possible while the callback is
// kept. Installed unconditionally rather than with `??=`, since a host that did
// provide an observer would take the slot and put the callback back out of
// reach — silently, and only on that host.
export const resizeObserverStubs: ResizeObserverStub[] = [];

class ResizeObserverStub implements ResizeObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();

  constructor(readonly callback: ResizeObserverCallback) {
    resizeObserverStubs.push(this);
  }

  /** Fire one resize tick. The entry list is empty: observers under test read
   *  the element, not the entry, because jsdom fills in no `contentRect`. */
  trigger(): void {
    this.callback([], this);
  }
}

globalThis.ResizeObserver = ResizeObserverStub;

// jsdom implements no CSS Font Loading API. Reading `document.fonts.ready`
// throws, and where that read sits inside a promise continuation the rejection
// lands in an unrelated `.catch` — the continuation is skipped and the test
// still passes (src/components/TerminalSurface.tsx waits for fonts before its
// reconciling fit).
Object.defineProperty(document, "fonts", {
  configurable: true,
  value: { ready: Promise.resolve({}) },
});

globalThis.matchMedia ??= ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
})) as typeof globalThis.matchMedia;

// jsdom stubs `scrollIntoView` out entirely; several views call it on mount.
Element.prototype.scrollIntoView ??= () => {};
