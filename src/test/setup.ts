import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, vi } from "vitest";

// Setup files run once per test file, so mocks registered here apply to every
// suite without each one re-declaring the same Tauri/browser scaffolding.

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
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

class TerminalStub {
  write = vi.fn();
  clear = vi.fn();
  dispose = vi.fn();
  reset = vi.fn();
  onData = vi.fn();
  loadAddon = vi.fn();
  open = vi.fn();
  cols = 80;
  rows = 24;
}

class FitAddonStub {
  fit = vi.fn();
  proposeDimensions = vi.fn();
}

vi.mock("@xterm/xterm", () => ({
  Terminal: TerminalStub,
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: FitAddonStub,
}));

// --- Browser APIs jsdom omits ----------------------------------------------
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

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
