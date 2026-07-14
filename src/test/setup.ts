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
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  convertFileSrc: vi.fn((path: string) => path),
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
