// McpConsentView — renders on the `mcp_consent_requested` event and routes
// Approve/Deny to `decideMcpConsent`. Mirrors the `listen()` capture pattern
// `useRunEvents.test.tsx` uses so the test drives the same subscription path
// `useTauriEvent` installs, rather than stubbing the hook itself.

import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const decideMcpConsent = vi.fn();
vi.mock("../lib/mcpGrants", () => ({
  decideMcpConsent: (...args: unknown[]) => decideMcpConsent(...args),
}));

const reportError = vi.fn();
vi.mock("../lib/errorBus", () => ({
  reportError: (...args: unknown[]) => reportError(...args),
}));

// Capture `listen` handlers so a test can dispatch a synthetic Tauri event.
const handlers: Record<string, Array<(e: { payload: unknown }) => void>> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => {
    (handlers[event] ??= []).push(cb);
    return Promise.resolve(() => {
      handlers[event] = (handlers[event] ?? []).filter((h) => h !== cb);
    });
  },
}));

import { McpConsentView } from "./McpConsentView";

function emit(event: string, payload: unknown) {
  for (const h of handlers[event] ?? []) h({ payload });
}

const CONSENT_REQUEST = {
  request_id: "req-1",
  client_name: "Claude Desktop",
  requested_scopes: ["read", "spend", "configure"],
  resource: "https://demeteo.local/mcp",
  redirect_uri: "http://127.0.0.1:9/cb",
};

afterEach(() => {
  for (const k of Object.keys(handlers)) delete handlers[k];
  decideMcpConsent.mockReset();
  reportError.mockReset();
});

async function mountAndEmit() {
  render(<McpConsentView />);
  await waitFor(() => expect(handlers.mcp_consent_requested?.length).toBeGreaterThan(0));
  act(() => emit("mcp_consent_requested", CONSENT_REQUEST));
}

describe("McpConsentView", () => {
  it("renders nothing until a consent request arrives", async () => {
    render(<McpConsentView />);
    await waitFor(() => expect(handlers.mcp_consent_requested?.length).toBeGreaterThan(0));
    expect(screen.queryByText(/requesting access/i)).not.toBeInTheDocument();
  });

  it("renders the client name and every requested scope on a McpConsentRequested-shaped event", async () => {
    await mountAndEmit();

    await waitFor(() => {
      expect(screen.getByText("Claude Desktop")).toBeInTheDocument();
    });
    expect(screen.getByText("read")).toBeInTheDocument();
    expect(screen.getByText("spend")).toBeInTheDocument();
    expect(screen.getByText("configure")).toBeInTheDocument();
    expect(screen.getByText("https://demeteo.local/mcp")).toBeInTheDocument();
    expect(screen.getByText("http://127.0.0.1:9/cb")).toBeInTheDocument();
  });

  it("shows the fixed 30-day expiry note", async () => {
    await mountAndEmit();

    await waitFor(() => {
      expect(screen.getByText(/30 days/)).toBeInTheDocument();
    });
  });

  it("calls decideMcpConsent(request_id, true) on Approve", async () => {
    await mountAndEmit();
    await waitFor(() => expect(screen.getByText("Claude Desktop")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /approve/i }));

    await waitFor(() => {
      expect(decideMcpConsent).toHaveBeenCalledWith("req-1", true);
    });
  });

  it("calls decideMcpConsent(request_id, false) on Deny", async () => {
    await mountAndEmit();
    await waitFor(() => expect(screen.getByText("Claude Desktop")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /deny/i }));

    await waitFor(() => {
      expect(decideMcpConsent).toHaveBeenCalledWith("req-1", false);
    });
  });

  it("clears the prompt after a decision resolves", async () => {
    decideMcpConsent.mockResolvedValue(undefined);
    await mountAndEmit();
    await waitFor(() => expect(screen.getByText("Claude Desktop")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /approve/i }));

    await waitFor(() => {
      expect(screen.queryByText("Claude Desktop")).not.toBeInTheDocument();
    });
  });

  it("reports the error and keeps the prompt open when the decision fails", async () => {
    decideMcpConsent.mockRejectedValue(new Error("backend unreachable"));
    await mountAndEmit();
    await waitFor(() => expect(screen.getByText("Claude Desktop")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /approve/i }));

    await waitFor(() => {
      expect(reportError).toHaveBeenCalled();
    });
    expect(screen.getByText("Claude Desktop")).toBeInTheDocument();
  });
});
