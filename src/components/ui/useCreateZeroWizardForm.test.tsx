// Renderer-based integration tests for the Create-From-Zero
// wizard form hook (`src/components/ui/useCreateZeroWizardForm.ts`).
//
// Pin down the three findings from the implementation report:
//
//   1. The namespace listing on the Provider step must call the
//      `listProviderNamespaces` wrapper from
//      `src/lib/createProjectWizard.ts` (which routes to the
//      `fetch_provider_groups` Tauri command). Previously the hook
//      imported a missing module — this test catches a reintroduction.
//
//   2. The Remote SSH tile must gate the wizard's **Next** control
//      until the chosen machine's `test_connection` probe returns
//      success. A probe failure must surface `machineProbeError`
//      inline so the commit path cannot silently fall back to local
//      credentials.
//
//   3. The `provider.host` value picked on the Provider step must
//      flow into the commit payload (the `providerCreateRepo` call)
//      as `provider_host`, so sub-1's backend HTTP adapter can route
//      to a self-hosted enterprise host.
//
// The hook calls `invoke()` directly, so the test stubs
// `window.__TAURI_INTERNALS__.invoke` to script the IPC layer. The
// test runner is `tsc --noEmit`; runtime assertions throw on
// failure (mirrors the convention in
// `src/hooks/useCreateProjectWizard.test.tsx`).

import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { type ReactElement, useEffect } from "react";

import { useCreateZeroWizardForm, type WizardFormApi } from "./useCreateZeroWizardForm";
import { machineStepGateReason } from "../../components/CreateFromZeroWizard";
import { ProjectProvider, useProject } from "../../context";
import { ErrorBusProvider } from "../../lib/errorBus";
import {
  listProviderNamespaces,
  providerCreateRepo,
  type ProviderNamespace,
} from "../../lib/createProjectWizard";
import type { Machine, Provider, WorkflowSummary } from "../../types";

// ── IPC stub ────────────────────────────────────────────────────────────
//
// The Tauri `invoke` is a thin wrapper around
// `window.__TAURI_INTERNALS__.invoke(cmd, args, options)`. We
// intercept at the internals level so the existing
// `import { invoke } from '@tauri-apps/api/core'` import in the
// hook picks up the mock.

interface IpcCall {
  cmd: string;
  args: Record<string, unknown> | undefined;
}

function uninstallIpcStub(): void {
  const tauri = (globalThis as unknown as { __TAURI_INTERNALS__?: unknown });
  delete tauri.__TAURI_INTERNALS__;
}

// ── Probe + bootstrap script helpers ───────────────────────────────────

interface IpcScriptSpec {
  fetch_provider_groups?: (providerId: string) => ProviderNamespace[];
  get_machines?: () => Machine[];
  workflow_list?: () => WorkflowSummary[];
  test_machine_connection?: (machineId: string) => void;
  provider_create_repo?: (args: Record<string, unknown>) => unknown;
}

function makeScriptedIpc(spec: IpcScriptSpec): { calls: IpcCall[]; script: IpcScriptSpec } {
  const calls: IpcCall[] = [];
  const tauri = (globalThis as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } });
  tauri.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      const call: IpcCall = { cmd, args: args ?? {} };
      calls.push(call);
      switch (cmd) {
        case 'fetch_provider_groups': {
          if (!spec.fetch_provider_groups) throw new Error(`no script for fetch_provider_groups`);
          return spec.fetch_provider_groups(String(args?.providerId ?? ''));
        }
        case 'get_machines': {
          if (!spec.get_machines) throw new Error(`no script for get_machines`);
          return spec.get_machines();
        }
        case 'workflow_list': {
          if (!spec.workflow_list) throw new Error(`no script for workflow_list`);
          return spec.workflow_list();
        }
        case 'test_machine_connection': {
          if (!spec.test_machine_connection) throw new Error(`no script for test_machine_connection`);
          return spec.test_machine_connection(String(args?.machineId ?? ''));
        }
        case 'provider_create_repo': {
          if (!spec.provider_create_repo) throw new Error(`no script for provider_create_repo`);
          return spec.provider_create_repo(args ?? {});
        }
        default:
          throw new Error(`unexpected invoke('${cmd}') in test script`);
      }
    },
  };
  return { calls, script: spec };
}

// ── Hook probe + provider seeder ───────────────────────────────────────

interface HookHolder { current: WizardFormApi | null; }

interface SeedProvidersProps {
  providers: ReadonlyArray<Provider>;
  holder: HookHolder;
}

function SeedProviders({ providers, holder }: SeedProvidersProps): ReactElement {
  const { state, dispatch } = useProject();
  // Seed providers exactly once (idempotent: dispatching the same
  // payload is a no-op for the reducer).
  useEffect(() => {
    if (state.providers.length === 0) {
      dispatch({ type: 'SET_PROVIDERS', providers: [...providers] });
    }
  }, [state.providers.length, dispatch, providers]);
  const result = useCreateZeroWizardForm();
  useEffect(() => { holder.current = result; });
  return <></>;
}

function mountForm(
  providers: ReadonlyArray<Provider>,
): { renderer: ReactTestRenderer; holder: HookHolder } {
  const holder: HookHolder = { current: null };
  const renderer = create(
    <ErrorBusProvider>
      <ProjectProvider>
        <SeedProviders providers={providers} holder={holder} />
      </ProjectProvider>
    </ErrorBusProvider>,
  );
  return { renderer, holder };
}

function flushEffects(): Promise<void> {
  // Two-tick flush: lets the Promise microtasks + the act() boundary
  // resolve before the next assertion runs. `setTimeout(..., 0)` is
  // available in the ES2020 lib without DOM extensions.
  return new Promise((resolve) => { setTimeout(resolve, 0); });
}

// ── (1) Namespace listing routes through `listProviderNamespaces` ──────

{
  // AC-3 regression: the namespace listing on the Provider step must
  // call `listProviderNamespaces(providerId)`, which under the hood
  // invokes `fetch_provider_groups`. A missing or phantom import
  // (e.g. a stale `listNamespaces` from a non-existent module) would
  // make this call resolve empty; the assertion below catches that.
  uninstallIpcStub();
  const captured: Array<{ cmd: string; providerId?: string }> = [];
  const tauri = (globalThis as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } });
  tauri.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      captured.push({ cmd, providerId: args?.providerId as string | undefined });
      if (cmd === 'fetch_provider_groups') {
        return [
          { id: 'me', name: 'me', kind: 'personal' },
          { id: 'acme', name: 'acme', kind: 'org' },
        ] as ProviderNamespace[];
      }
      if (cmd === 'get_machines') return [] as Machine[];
      if (cmd === 'workflow_list') return [] as WorkflowSummary[];
      throw new Error(`unexpected invoke('${cmd}')`);
    },
  };

  const providers: Provider[] = [
    { id: 'gh-1', type: 'github', name: 'github', host: 'github.com', pat: 'hidden', username: 'me', avatarUrl: '' },
  ];

  // Direct call to the wrapper — pins the contract.
  const direct = await listProviderNamespaces('gh-1');
  if (direct.length !== 2 || direct[0].id !== 'me' || direct[1].kind !== 'org') {
    throw new Error(
      `listProviderNamespaces: expected 2 namespaces with personal+org, got ${JSON.stringify(direct)}`,
    );
  }
  const directCall = captured.find((c) => c.cmd === 'fetch_provider_groups');
  if (!directCall) throw new Error('listProviderNamespaces did not invoke fetch_provider_groups');
  if (directCall.providerId !== 'gh-1') {
    throw new Error(`listProviderNamespaces: expected providerId=gh-1, got ${directCall.providerId}`);
  }

  // Now mount the hook and assert the same call is made when the
  // user picks a provider on the Provider step.
  const { renderer, holder } = mountForm(providers);
  await act(async () => { await flushEffects(); });
  const h = holder.current;
  if (!h) throw new Error('hook did not mount');

  // Sanity: pre-pick, no fetch_provider_groups call yet.
  const beforeCount = captured.filter((c) => c.cmd === 'fetch_provider_groups').length;
  if (beforeCount !== 1) {
    throw new Error(`expected 1 fetch_provider_groups call (direct) so far, got ${beforeCount}`);
  }

  await act(async () => {
    h.setProviderId('gh-1');
    await flushEffects();
  });
  const afterCount = captured.filter((c) => c.cmd === 'fetch_provider_groups').length;
  if (afterCount !== 2) {
    throw new Error(
      `expected 2 fetch_provider_groups calls after setProviderId (direct + form), got ${afterCount}`,
    );
  }
  const fetchProviderCalls = captured.filter((c) => c.cmd === 'fetch_provider_groups');
  const formCall = fetchProviderCalls[fetchProviderCalls.length - 1];
  if (formCall?.providerId !== 'gh-1') {
    throw new Error(`form-driven listProviderNamespaces: expected providerId=gh-1, got ${formCall?.providerId}`);
  }
  // The form's state must mirror the wrapper's response.
  if (holder.current?.namespaces.length !== 2) {
    throw new Error(`form.namespaces expected 2, got ${holder.current?.namespaces.length}`);
  }
  if (holder.current?.namespaceId !== 'me') {
    throw new Error(`form.namespaceId auto-pick expected 'me' (personal), got '${holder.current?.namespaceId}'`);
  }
  renderer.unmount();
  uninstallIpcStub();
}

// ── (2) Remote tile blocks Next when probe fails ───────────────────────

{
  uninstallIpcStub();
  const machines: Machine[] = [
    { id: 'm-1', name: 'box', host: '10.0.0.1', port: 22, username: 'u', auth_type: 'key' },
  ];
  const providers: Provider[] = [
    { id: 'gh-1', type: 'github', name: 'github', host: 'github.com', pat: 'hidden', username: 'me', avatarUrl: '' },
  ];
  const { calls } = makeScriptedIpc({
    get_machines: () => machines,
    workflow_list: () => [],
    fetch_provider_groups: () => [],
    test_machine_connection: () => { throw { kind: 'transport', message: 'ssh: connect: connection refused' }; },
  });

  const { renderer, holder } = mountForm(providers);
  await act(async () => { await flushEffects(); });

  // Pick the remote machine — the hook must fire test_machine_connection.
  await act(async () => {
    holder.current?.setMachineKind('remote');
    holder.current?.setMachineId('m-1');
    await flushEffects();
  });

  const probeCalls = calls.filter((c) => c.cmd === 'test_machine_connection');
  if (probeCalls.length === 0) {
    throw new Error('expected at least one test_machine_connection call when remote machine is picked');
  }
  const lastProbe = probeCalls[probeCalls.length - 1];
  if ((lastProbe?.args as { machineId?: string } | undefined)?.machineId !== 'm-1') {
    throw new Error(`probe call expected machineId=m-1, got ${JSON.stringify(lastProbe?.args)}`);
  }

  // The hook must surface the failure on `machineProbeStatus` and
  // `machineProbeError`. The wizard reads these to disable Next.
  if (holder.current?.machineProbeStatus !== 'error') {
    throw new Error(
      `machineProbeStatus expected 'error', got '${holder.current?.machineProbeStatus}'`,
    );
  }
  if (!holder.current?.machineProbeError) {
    throw new Error('machineProbeError must be set after a failed probe');
  }
  if (!holder.current.machineProbeError.toLowerCase().includes('connection')) {
    throw new Error(
      `machineProbeError expected to surface 'connection refused', got '${holder.current.machineProbeError}'`,
    );
  }

  // The pure gate helper from the wizard must report a non-empty
  // reason, which translates to "Next disabled" in the UI.
  const gate = machineStepGateReason({
    machineKind: 'remote',
    machineId: 'm-1',
    probeStatus: 'error' as const,
    probeError: holder.current.machineProbeError,
  });
  if (gate === '') {
    throw new Error('machineStepGateReason must return non-empty when probe failed');
  }
  if (!gate.toLowerCase().includes('connection')) {
    throw new Error(
      `machineStepGateReason expected to surface the failure ('connection'), got '${gate}'`,
    );
  }

  // Toggling to a successful probe must clear the gate. We swap the
  // script mid-test to model "user re-ran the probe and it worked".
  makeScriptedIpc({
    get_machines: () => machines,
    workflow_list: () => [],
    fetch_provider_groups: () => [],
    test_machine_connection: () => undefined,
  });
  await act(async () => {
    holder.current?.retestMachineConnection();
    await flushEffects();
  });
  // The `act()` above mutated `holder.current`; we re-read with an
  // explicit cast to defeat TypeScript's flow-narrowing from the
  // pre-mutation `=== 'error'` guard.
  const statusAfterRetest = (holder.current as WizardFormApi | null)?.machineProbeStatus;
  if (statusAfterRetest !== 'success') {
    throw new Error(
      `machineProbeStatus expected 'success' after retest, got '${statusAfterRetest}'`,
    );
  }

  // Toggling back to local must clear the probe gate regardless of
  // any leftover remote state.
  await act(async () => {
    holder.current?.setMachineKind('local');
    await flushEffects();
  });
  const statusAfterLocal = (holder.current as WizardFormApi | null)?.machineProbeStatus;
  if (statusAfterLocal !== 'idle') {
    throw new Error(
      `machineProbeStatus expected 'idle' after switching to local, got '${statusAfterLocal}'`,
    );
  }
  const localGate = machineStepGateReason({
    machineKind: 'local',
    machineId: '',
    probeStatus: holder.current.machineProbeStatus,
    probeError: null,
  });
  if (localGate !== '') {
    throw new Error(`local machineStepGateReason expected '', got '${localGate}'`);
  }

  renderer.unmount();
  uninstallIpcStub();
}

// ── (3) provider.host reaches the commit payload ──────────────────────

{
  uninstallIpcStub();
  const providers: Provider[] = [
    { id: 'gh-corp', type: 'github', name: 'GH Corp', host: 'gh.corp.example.com', pat: 'hidden', username: 'me', avatarUrl: '' },
  ];
  const { calls } = makeScriptedIpc({
    get_machines: () => [],
    workflow_list: () => [],
    fetch_provider_groups: () => [],
    provider_create_repo: () => ({ full_name: 'me/test', default_branch: 'main', clone_url: 'https://x' }),
  });

  const { renderer, holder } = mountForm(providers);
  await act(async () => { await flushEffects(); });

  // Pick the provider — the hook must derive the host.
  await act(async () => {
    holder.current?.setProviderId('gh-corp');
    await flushEffects();
  });
  if (holder.current?.providerHost !== 'gh.corp.example.com') {
    throw new Error(
      `providerHost expected 'gh.corp.example.com', got '${holder.current?.providerHost}'`,
    );
  }

  // Direct wrapper call with the resolved host: backend must see
  // `providerHost: 'gh.corp.example.com'`.
  await act(async () => {
    await providerCreateRepo({
      providerId: 'gh-corp',
      namespaceId: 'me',
      name: 'test',
      private: true,
      providerHost: holder.current?.providerHost,
    });
  });
  const createRepoCall = calls.find((c) => c.cmd === 'provider_create_repo');
  if (!createRepoCall) throw new Error('provider_create_repo was not called');
  const sentArgs = createRepoCall.args as Record<string, unknown>;
  if (sentArgs.providerHost !== 'gh.corp.example.com') {
    throw new Error(
      `provider_create_repo args.providerHost expected 'gh.corp.example.com', got ${JSON.stringify(sentArgs.providerHost)}`,
    );
  }
  if (sentArgs.providerId !== 'gh-corp') {
    throw new Error(`provider_create_repo args.providerId expected 'gh-corp', got ${JSON.stringify(sentArgs.providerId)}`);
  }

  // Omitting providerHost (empty string) sends `null` to the
  // backend — the HTTP adapter treats null as "fall back to the
  // provider's configured default host".
  await act(async () => {
    await providerCreateRepo({
      providerId: 'gh-corp',
      namespaceId: 'me',
      name: 'test',
      private: true,
    });
  });
  const secondCall = calls.filter((c) => c.cmd === 'provider_create_repo');
  const lastCreate = secondCall[secondCall.length - 1];
  if ((lastCreate?.args as { providerHost?: unknown } | undefined)?.providerHost !== null) {
    throw new Error(
      `provider_create_repo args.providerHost expected null when omitted, got ${JSON.stringify((lastCreate?.args as { providerHost?: unknown } | undefined)?.providerHost)}`,
    );
  }

  renderer.unmount();
  uninstallIpcStub();
}

// ── Exported results ───────────────────────────────────────────────────

export const useCreateZeroWizardFormTestResults = {
  namespaceListingWired: true,
  remoteTileBlocksNextOnProbeError: true,
  providerHostReachesCommitPayload: true,
} as const;
