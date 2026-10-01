import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { ErrorBusProvider, reportError, useErrorBus } from '../lib/errorBus';
import { ErrorToast } from './ErrorToast';

function renderStack() {
  render(
    <ErrorBusProvider>
      <ErrorToast />
    </ErrorBusProvider>,
  );
}

afterEach(() => {
  const { clear } = renderHook(() => useErrorBus()).result.current;
  act(() => clear());
});

describe('ErrorToast', () => {
  it('renders a reported action and dismisses the toast once it runs', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    const onClick = vi.fn();
    renderStack();

    act(() => {
      reportError(
        { kind: 'runner_incompatible', message: 'upgrade the runner' },
        { action: { label: 'Open machine settings', onClick } },
      );
    });
    fireEvent.click(screen.getByRole('button', { name: 'Open machine settings' }));

    expect(onClick).toHaveBeenCalledOnce();
    expect(screen.queryByTestId('error-toast-runner_incompatible')).toBeNull();
  });

  it('renders no action button for a toast reported without one', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {});
    renderStack();

    act(() => {
      reportError({ kind: 'runner_incompatible', message: 'upgrade the runner' });
    });

    expect(screen.getByTestId('error-toast-runner_incompatible')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Open machine settings' })).toBeNull();
  });
});
