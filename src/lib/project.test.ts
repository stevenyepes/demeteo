// Unit tests for `deriveMachineId` / `checkWorkspaceLiveness` in
// `src/lib/project.ts`.
//
// The wrapper resolves connectivity purely client-side: it derives a
// `machineId` from the project's `compute_type`/`remote_host` and calls the
// existing `test_machine_connection` Tauri command (already used by
// `MachinesView.tsx` and `useCreateZeroWizardForm.ts`) — no bespoke
// liveness command on the Rust side.
//
// Runner: `tsc --noEmit`. Assertions throw on failure (repo convention,
// see `src/lib/shortcuts.test.ts`).

import { checkWorkspaceLiveness, deriveMachineId } from './project';

interface IpcCall {
  cmd: string;
  args: Record<string, unknown> | undefined;
}

function installIpcStub(handler: (cmd: string, args: Record<string, unknown> | undefined) => unknown): { calls: IpcCall[] } {
  const calls: IpcCall[] = [];
  const tauri = (globalThis as unknown as {
    __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> };
  });
  tauri.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      calls.push({ cmd, args });
      if (cmd !== 'test_machine_connection') throw new Error(`unexpected command: ${cmd}`);
      return handler(cmd, args);
    },
  };
  return { calls };
}

function uninstallIpcStub(): void {
  delete (globalThis as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
}

async function run() {
  // ── deriveMachineId ──────────────────────────────────────────────

  // (1) missing compute_type -> 'local'
  {
    const id = deriveMachineId({});
    if (id !== 'local') throw new Error(`expected 'local', got '${id}'`);
  }

  // (2) compute_type === 'local' (case-insensitive) -> 'local'
  {
    const id = deriveMachineId({ compute_type: 'LOCAL', remote_host: 'should-be-ignored' });
    if (id !== 'local') throw new Error(`expected 'local', got '${id}'`);
  }

  // (3) remote compute_type -> remote_host
  {
    const id = deriveMachineId({ compute_type: 'remote', remote_host: 'box-1' });
    if (id !== 'box-1') throw new Error(`expected 'box-1', got '${id}'`);
  }

  // (4) remote compute_type with no remote_host -> 'local' fallback
  {
    const id = deriveMachineId({ compute_type: 'remote', remote_host: null });
    if (id !== 'local') throw new Error(`expected 'local', got '${id}'`);
  }

  // ── checkWorkspaceLiveness ───────────────────────────────────────

  // (5) local project (no compute_type/remote_host) resolves online via machineId: 'local'
  {
    const { calls } = installIpcStub(() => undefined);
    const result = await checkWorkspaceLiveness({ id: 'proj-1' });
    if (calls.length !== 1) {
      throw new Error(`expected exactly 1 invoke call, got ${calls.length}`);
    }
    if (calls[0].cmd !== 'test_machine_connection') {
      throw new Error(`expected command 'test_machine_connection', got '${calls[0].cmd}'`);
    }
    if (calls[0].args?.machineId !== 'local') {
      throw new Error(`expected args.machineId === 'local', got ${JSON.stringify(calls[0].args)}`);
    }
    if (result.project_id !== 'proj-1' || result.liveness !== 'online' || typeof result.checked_at !== 'string') {
      throw new Error(`expected online result, got ${JSON.stringify(result)}`);
    }
    uninstallIpcStub();
  }

  // (6) remote project resolves via its remote_host as machineId
  {
    const { calls } = installIpcStub(() => undefined);
    const result = await checkWorkspaceLiveness({ id: 'proj-2', compute_type: 'remote', remote_host: 'box-42' });
    if (calls[0].args?.machineId !== 'box-42') {
      throw new Error(`expected args.machineId === 'box-42', got ${JSON.stringify(calls[0].args)}`);
    }
    if (result.project_id !== 'proj-2' || result.liveness !== 'online') {
      throw new Error(`expected online result, got ${JSON.stringify(result)}`);
    }
    uninstallIpcStub();
  }

  // (7) a rejected invoke resolves with liveness: 'offline' rather than throwing
  {
    installIpcStub(() => {
      throw new Error('ssh: connect: connection refused');
    });
    const result = await checkWorkspaceLiveness({ id: 'proj-3', compute_type: 'remote', remote_host: 'box-99' });
    if (result.project_id !== 'proj-3' || result.liveness !== 'offline' || typeof result.checked_at !== 'string') {
      throw new Error(`expected offline result, got ${JSON.stringify(result)}`);
    }
    uninstallIpcStub();
  }
}

export const projectLivenessTestResults = run();
