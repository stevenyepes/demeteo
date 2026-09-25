import { invoke } from '@tauri-apps/api/core';
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { getAgentModels } from '../lib/agentModels';
import type { AgentAvailability } from '../lib/featureDetail';
import { useRunChoice } from './useRunChoice';

const { reportError } = vi.hoisted(() => ({ reportError: vi.fn() }));

vi.mock('../lib/agentModels', () => ({ getAgentModels: vi.fn() }));
vi.mock('../lib/errorBus', () => ({
  useErrorBus: () => ({ toasts: [], reportError, dismiss: vi.fn(), clear: vi.fn() }),
}));

const CATALOG = [
  { kind: 'claude-code', display_label: 'Claude Code', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'medium', 'high', 'xhigh', 'max'] },
  { kind: 'codex', display_label: 'Codex', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'medium', 'high', 'xhigh'] },
  { kind: 'hermes', display_label: 'Hermes', lists_models: false, default_model: null, install_command: '', effort_levels: [] },
  { kind: 'opencode', display_label: 'opencode', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'medium', 'high', 'xhigh', 'max'] },
];

/** One machine's answer for every registered harness: two usable, one switched
 *  off in settings, one never installed. */
const MACHINE_AGENTS: AgentAvailability[] = [
  { kind: 'claude-code', enabled: true, available: true },
  { kind: 'codex', enabled: true, available: true },
  { kind: 'hermes', enabled: true, available: true },
  { kind: 'opencode', enabled: false, available: true },
  { kind: 'pi', enabled: true, available: false },
];

const MODELS = [
  { value: 'gpt-5.6-codex', name: 'GPT-5.6 Codex' },
  { value: 'o5-mini', name: 'o5 mini' },
];

function mount(machineAgents: AgentAvailability[] = MACHINE_AGENTS) {
  return renderHook(() => useRunChoice({ machineAgents, machineId: 'm-1' }));
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd === 'list_agents' ? Promise.resolve(CATALOG) : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
  vi.mocked(getAgentModels).mockResolvedValue(MODELS);
});

describe('useRunChoice', () => {
  it('reads an untouched surface as inherit, not as a pinned empty string', async () => {
    const { result } = mount();
    await act(async () => {});

    expect([result.current.workflowId, result.current.agentKind, result.current.model, result.current.effort]).toEqual(['', '', '', '']);
    expect(result.current.choice).toEqual({
      workflowId: undefined,
      agentKind: undefined,
      model: undefined,
      effort: undefined,
    });
    expect(getAgentModels).not.toHaveBeenCalled();
  });

  it('offers only the harnesses this machine has enabled and installed', async () => {
    const { result } = mount();
    await act(async () => {});

    expect(result.current.availableAgents).toEqual(['claude-code', 'codex', 'hermes']);
  });

  it('clears the pinned model, re-probes and clamps the effort when the harness changes', async () => {
    const { result } = mount();
    await act(async () => {});

    act(() => {
      result.current.setAgentKind('claude-code');
      result.current.setEffort('max');
    });
    await waitFor(() => expect(result.current.availableModels).toEqual(MODELS));
    act(() => result.current.setModel('claude-opus-5'));
    expect(result.current.choice.model).toBe('claude-opus-5');

    vi.mocked(getAgentModels).mockResolvedValue([{ value: 'gpt-5.6', name: 'GPT-5.6' }]);
    await act(async () => {
      result.current.setAgentKind('codex');
    });

    expect(getAgentModels).toHaveBeenLastCalledWith('m-1', 'codex');
    expect(result.current.model).toBe('');
    // codex declares no `max`, so a level the previous harness accepted is
    // clamped down rather than silently re-sent.
    expect(result.current.effort).toBe('xhigh');
    expect(result.current.choice).toEqual({
      workflowId: undefined,
      agentKind: 'codex',
      model: undefined,
      effort: 'xhigh',
    });
    expect(result.current.availableModels).toEqual([{ value: 'gpt-5.6', name: 'GPT-5.6' }]);
  });

  it('reports no effort levels for a harness that declares none', async () => {
    const { result } = mount();
    await act(async () => {});
    expect(result.current.effortLevels).toEqual(['low', 'medium', 'high', 'xhigh', 'max']);

    act(() => result.current.setEffort('high'));
    await act(async () => {
      result.current.setAgentKind('hermes');
    });

    expect(result.current.effortLevels).toEqual([]);
    expect(result.current.effort).toBe('');
    expect(result.current.choice.effort).toBeUndefined();
  });

  it('keeps a failed probe off the surface and out of the way', async () => {
    vi.mocked(getAgentModels).mockRejectedValue(new Error('no agent on m-1'));
    const { result } = mount();
    await act(async () => {});

    await act(async () => {
      result.current.setAgentKind('codex');
    });

    expect(result.current.availableModels).toEqual([]);
    expect(result.current.isLoadingModels).toBe(false);
    expect(result.current.agentKind).toBe('codex');
    expect(reportError).toHaveBeenCalled();
  });

  it('returns the same object across a render that changed nothing in it', async () => {
    const { result, rerender } = mount();
    await act(async () => {});

    const before = result.current;
    rerender();
    const after = result.current;

    expect(after).toBe(before);
    for (const key of Object.keys(before) as Array<keyof typeof before>) {
      expect(after[key]).toBe(before[key]);
    }
  });
});
