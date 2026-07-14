// Integration tests for the Create-From-Zero wizard form hook
// (`src/components/ui/useCreateZeroWizardForm.ts`).
//
// These pin the three findings from the implementation report:
//
//   1. The namespace listing on the Provider step must go through the
//      `listProviderNamespaces` wrapper in `src/lib/createProjectWizard.ts`
//      (which invokes `fetch_provider_groups`). The hook once imported a
//      module that did not exist; this catches a reintroduction.
//
//   2. The Remote SSH tile must gate the wizard's Next control until the chosen
//      machine's `test_connection` probe succeeds. A failed probe surfaces
//      inline so the commit path cannot silently fall back to local credentials.
//
//   3. The `provider.host` picked on the Provider step must reach the commit
//      payload as `provider_host`, so the backend HTTP adapter can route to a
//      self-hosted enterprise host.
//
// `invoke` is mocked globally in `src/test/setup.ts`; each test scripts it with
// a per-command router.

import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { useEffect, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useCreateZeroWizardForm } from "./useCreateZeroWizardForm";
import { machineStepGateReason } from "../../components/CreateFromZeroWizard";
import { ProjectProvider, useProject } from "../../context";
import { ErrorBusProvider } from "../../lib/errorBus";
import {
  listProviderNamespaces,
  providerCreateRepo,
  type ProviderNamespace,
} from "../../lib/createProjectWizard";
import type { Machine, Provider, WorkflowSummary } from "../../types";

type IpcHandlers = Partial<{
  fetch_provider_groups: (args: Record<string, unknown>) => ProviderNamespace[];
  get_machines: () => Machine[];
  workflow_list: () => WorkflowSummary[];
  test_machine_connection: (args: Record<string, unknown>) => void;
  provider_create_repo: (args: Record<string, unknown>) => unknown;
}>;

const mockedInvoke = vi.mocked(invoke);

// Routes `invoke(cmd, args)` to the scripted handler. Anything unscripted
// throws, so a stray command shows up as a failure rather than a silent
// `undefined`.
function scriptIpc(handlers: IpcHandlers) {
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    const handler = handlers[cmd as keyof IpcHandlers];
    if (!handler) throw new Error(`unscripted invoke('${cmd}')`);
    return handler((args ?? {}) as never);
  }) as typeof invoke);
}

function callsTo(cmd: string): Record<string, unknown>[] {
  return mockedInvoke.mock.calls
    .filter(([name]) => name === cmd)
    .map(([, args]) => (args ?? {}) as Record<string, unknown>);
}

// `Array.prototype.at` needs the ES2022 lib; this project targets ES2020.
function lastCallTo(cmd: string): Record<string, unknown> | undefined {
  const calls = callsTo(cmd);
  return calls[calls.length - 1];
}

const GITHUB: Provider = {
  id: "gh-1",
  type: "github",
  name: "github",
  host: "github.com",
  pat: "hidden",
  username: "me",
  avatarUrl: "",
};

const GITHUB_ENTERPRISE: Provider = {
  ...GITHUB,
  id: "gh-corp",
  name: "GH Corp",
  host: "gh.corp.example.com",
};

const MACHINE: Machine = {
  id: "m-1",
  name: "box",
  host: "10.0.0.1",
  port: 22,
  username: "u",
  auth_type: "key",
};

// Seeds the ProjectContext with providers before the hook reads them.
function wrapperWith(providers: ReadonlyArray<Provider>) {
  function SeedProviders({ children }: { children: ReactNode }) {
    const { state, dispatch } = useProject();
    useEffect(() => {
      if (state.providers.length === 0) {
        dispatch({ type: "SET_PROVIDERS", providers: [...providers] });
      }
    }, [state.providers.length, dispatch]);
    return <>{children}</>;
  }

  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <ErrorBusProvider>
        <ProjectProvider>
          <SeedProviders>{children}</SeedProviders>
        </ProjectProvider>
      </ErrorBusProvider>
    );
  };
}

function mountForm(providers: ReadonlyArray<Provider>) {
  return renderHook(() => useCreateZeroWizardForm(), { wrapper: wrapperWith(providers) });
}

beforeEach(() => {
  mockedInvoke.mockReset();
});

// AC-3 regression: a phantom import would make this call resolve empty.
describe("(1) namespace listing routes through listProviderNamespaces", () => {
  const NAMESPACES: ProviderNamespace[] = [
    { id: "me", name: "me", kind: "personal" },
    { id: "acme", name: "acme", kind: "org" },
  ];

  beforeEach(() => {
    scriptIpc({
      fetch_provider_groups: () => NAMESPACES,
      get_machines: () => [],
      workflow_list: () => [],
    });
  });

  it("invokes fetch_provider_groups with the provider id", async () => {
    const namespaces = await listProviderNamespaces("gh-1");

    expect(namespaces).toEqual(NAMESPACES);
    expect(callsTo("fetch_provider_groups")).toEqual([{ providerId: "gh-1" }]);
  });

  it("lists namespaces and auto-picks the personal one when a provider is chosen", async () => {
    const { result } = mountForm([GITHUB]);

    expect(callsTo("fetch_provider_groups")).toHaveLength(0);

    await act(async () => {
      result.current.setProviderId("gh-1");
    });

    await waitFor(() => {
      expect(result.current.namespaces).toHaveLength(2);
    });

    expect(callsTo("fetch_provider_groups")).toEqual([{ providerId: "gh-1" }]);
    expect(result.current.namespaceId).toBe("me");
  });
});

describe("(2) the remote tile gates Next on the connection probe", () => {
  it("surfaces a failed probe and blocks the step", async () => {
    scriptIpc({
      get_machines: () => [MACHINE],
      workflow_list: () => [],
      fetch_provider_groups: () => [],
      test_machine_connection: () => {
        throw { kind: "transport", message: "ssh: connect: connection refused" };
      },
    });

    const { result } = mountForm([GITHUB]);

    await act(async () => {
      result.current.setMachineKind("remote");
      result.current.setMachineId("m-1");
    });

    await waitFor(() => {
      expect(result.current.machineProbeStatus).toBe("error");
    });

    expect(lastCallTo("test_machine_connection")).toMatchObject({ machineId: "m-1" });
    expect(result.current.machineProbeError?.toLowerCase()).toContain("connection");

    // The pure gate helper is what the wizard reads to disable Next.
    const gate = machineStepGateReason({
      machineKind: "remote",
      machineId: "m-1",
      probeStatus: "error",
      probeError: result.current.machineProbeError,
    });

    expect(gate).not.toBe("");
    expect(gate.toLowerCase()).toContain("connection");
  });

  it("clears the gate once the probe succeeds on retest", async () => {
    let probeFails = true;
    scriptIpc({
      get_machines: () => [MACHINE],
      workflow_list: () => [],
      fetch_provider_groups: () => [],
      test_machine_connection: () => {
        if (probeFails) throw { kind: "transport", message: "connection refused" };
      },
    });

    const { result } = mountForm([GITHUB]);

    await act(async () => {
      result.current.setMachineKind("remote");
      result.current.setMachineId("m-1");
    });
    await waitFor(() => {
      expect(result.current.machineProbeStatus).toBe("error");
    });

    // Model "the user fixed the box and re-ran the probe".
    probeFails = false;
    await act(async () => {
      result.current.retestMachineConnection();
    });

    await waitFor(() => {
      expect(result.current.machineProbeStatus).toBe("success");
    });
  });

  it("drops the probe gate entirely when switching back to local", async () => {
    scriptIpc({
      get_machines: () => [MACHINE],
      workflow_list: () => [],
      fetch_provider_groups: () => [],
      test_machine_connection: () => {
        throw { kind: "transport", message: "connection refused" };
      },
    });

    const { result } = mountForm([GITHUB]);

    await act(async () => {
      result.current.setMachineKind("remote");
      result.current.setMachineId("m-1");
    });
    await waitFor(() => {
      expect(result.current.machineProbeStatus).toBe("error");
    });

    await act(async () => {
      result.current.setMachineKind("local");
    });

    await waitFor(() => {
      expect(result.current.machineProbeStatus).toBe("idle");
    });

    expect(
      machineStepGateReason({
        machineKind: "local",
        machineId: "",
        probeStatus: result.current.machineProbeStatus,
        probeError: null,
      }),
    ).toBe("");
  });
});

describe("(3) provider.host reaches the commit payload", () => {
  beforeEach(() => {
    scriptIpc({
      get_machines: () => [],
      workflow_list: () => [],
      fetch_provider_groups: () => [],
      provider_create_repo: () => ({
        full_name: "me/test",
        default_branch: "main",
        clone_url: "https://x",
      }),
    });
  });

  it("derives the host from the picked provider", async () => {
    const { result } = mountForm([GITHUB_ENTERPRISE]);

    await act(async () => {
      result.current.setProviderId("gh-corp");
    });

    await waitFor(() => {
      expect(result.current.providerHost).toBe("gh.corp.example.com");
    });
  });

  it("forwards the enterprise host to provider_create_repo", async () => {
    await providerCreateRepo({
      providerId: "gh-corp",
      namespaceId: "me",
      name: "test",
      private: true,
      providerHost: "gh.corp.example.com",
    });

    expect(lastCallTo("provider_create_repo")).toMatchObject({
      providerId: "gh-corp",
      providerHost: "gh.corp.example.com",
    });
  });

  // The HTTP adapter reads null as "fall back to the provider's default host",
  // so an omitted host must not arrive as `undefined`.
  it("sends null when the host is omitted", async () => {
    await providerCreateRepo({
      providerId: "gh-corp",
      namespaceId: "me",
      name: "test",
      private: true,
    });

    expect(lastCallTo("provider_create_repo")?.providerHost).toBeNull();
  });
});
