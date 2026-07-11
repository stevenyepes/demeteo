// Unit tests for `checkWorkspaceLiveness` in `src/lib/project.ts`.
//
// The wrapper must call the `check_workspace_liveness` Tauri command with
// a camelCase `projectId` arg (matching `src-tauri/src/commands/project.rs`)
// and resolve with the `LivenessResult` payload verbatim.
//
// Runner: `tsc --noEmit`. Assertions throw on failure (repo convention,
// see `src/lib/shortcuts.test.ts`).

import { checkWorkspaceLiveness } from './project';

interface IpcCall {
  cmd: string;
  args: Record<string, unknown> | undefined;
}

function installIpcStub(result: unknown): { calls: IpcCall[] } {
  const calls: IpcCall[] = [];
  const tauri = (globalThis as unknown as {
    __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> };
  });
  tauri.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      calls.push({ cmd, args });
      if (cmd !== 'check_workspace_liveness') throw new Error(`unexpected command: ${cmd}`);
      return result;
    },
  };
  return { calls };
}

function uninstallIpcStub(): void {
  delete (globalThis as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
}

async function run() {
  // ── (1) calls the correct command with camelCase projectId ──
  {
    const payload = { project_id: 'proj-1', liveness: 'online', checked_at: '2026-07-11T00:00:00Z' };
    const { calls } = installIpcStub(payload);
    const result = await checkWorkspaceLiveness('proj-1');
    if (calls.length !== 1) {
      throw new Error(`expected exactly 1 invoke call, got ${calls.length}`);
    }
    if (calls[0].cmd !== 'check_workspace_liveness') {
      throw new Error(`expected command 'check_workspace_liveness', got '${calls[0].cmd}'`);
    }
    if (calls[0].args?.projectId !== 'proj-1') {
      throw new Error(`expected args.projectId === 'proj-1', got ${JSON.stringify(calls[0].args)}`);
    }
    if (result.project_id !== 'proj-1' || result.liveness !== 'online' || result.checked_at !== '2026-07-11T00:00:00Z') {
      throw new Error(`expected result to match payload verbatim, got ${JSON.stringify(result)}`);
    }
    uninstallIpcStub();
  }

  // ── (2) 'offline' liveness passes through untouched ──
  {
    const payload = { project_id: 'proj-2', liveness: 'offline', checked_at: '2026-07-11T00:05:00Z' };
    installIpcStub(payload);
    const result = await checkWorkspaceLiveness('proj-2');
    if (result.liveness !== 'offline') {
      throw new Error(`expected liveness === 'offline', got '${result.liveness}'`);
    }
    uninstallIpcStub();
  }
}

export const projectLivenessTestResults = run();
