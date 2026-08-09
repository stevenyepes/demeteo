/**
 * The claim: a retry can be re-pinned before it is fired, and each control says
 * what leaving it alone means.
 *
 * The wording is the assertion rather than the markup because these three
 * selects are the only place a user can answer "the harness is wrong, run it on
 * the other one" — a blank placeholder there reads as "no harness", which is a
 * different and untrue statement from "keep the one the feature already runs".
 */
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { RerunOptions } from './RerunOptions';
import type { HarnessOverrides } from './useHarnessOverrides';

function overrides(over: Partial<HarnessOverrides> = {}): HarnessOverrides {
  return {
    availableModels: [{ value: 'sonnet', name: 'Claude Sonnet' }],
    selectedModel: '',
    setSelectedModel: vi.fn(),
    isLoadingModels: false,
    availableAgents: ['opencode', 'claude-code'],
    selectedAgent: '',
    selectedEffort: '',
    setSelectedEffort: vi.fn(),
    featureAgentKind: 'opencode',
    retryEffortLevels: ['low', 'high'],
    onAgentChange: vi.fn(),
    adoptFeatureModel: vi.fn(),
    probeForFeature: vi.fn(),
    ...over,
  };
}

afterEach(cleanup);

describe('RerunOptions', () => {
  it('offers the harnesses the machine actually has and reports a switch', () => {
    const onAgentChange = vi.fn();
    render(<RerunOptions overrides={overrides({ onAgentChange })} />);

    const harness = screen.getByLabelText('Harness');
    expect(screen.getByRole('option', { name: 'claude code' })).toBeInTheDocument();
    fireEvent.change(harness, { target: { value: 'claude-code' } });
    expect(onAgentChange).toHaveBeenCalledWith('claude-code');
  });

  it('names the feature harness as what leaving the control alone keeps', () => {
    render(<RerunOptions overrides={overrides({ featureAgentKind: 'claude-code' })} />);
    expect(screen.getByRole('option', { name: 'Default (claude code)' })).toBeInTheDocument();
  });

  it('says it is probing rather than offering a model list it has not got', () => {
    render(<RerunOptions overrides={overrides({ isLoadingModels: true, availableModels: [] })} />);
    expect(screen.getByText(/probing models/i)).toBeInTheDocument();
    expect(screen.queryByLabelText('Model')).not.toBeInTheDocument();
  });

  it('re-pins the model on a choice', () => {
    const setSelectedModel = vi.fn();
    render(<RerunOptions overrides={overrides({ setSelectedModel })} />);
    fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'sonnet' } });
    expect(setSelectedModel).toHaveBeenCalledWith('sonnet');
  });

  it('greys effort out for a harness with no per-invocation control', () => {
    render(<RerunOptions overrides={overrides({ retryEffortLevels: [], featureAgentKind: 'hermes' })} />);
    const effort = screen.getByLabelText('Effort');
    expect(effort).toBeDisabled();
    expect(effort).toHaveAttribute('title', expect.stringContaining('does not support effort'));
  });

  it('re-pins the effort on a choice', () => {
    const setSelectedEffort = vi.fn();
    render(<RerunOptions overrides={overrides({ setSelectedEffort })} />);
    fireEvent.change(screen.getByLabelText('Effort'), { target: { value: 'high' } });
    expect(setSelectedEffort).toHaveBeenCalledWith('high');
  });
});
