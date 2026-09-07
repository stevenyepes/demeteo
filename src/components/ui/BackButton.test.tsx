// The one Back control. What is worth pinning here is not that it renders an
// arrow — it is that it *pops* rather than navigating to a destination of its
// own, which is the whole difference from the five buttons it replaced.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect } from 'react';
import { describe, expect, it } from 'vitest';

import { NavigationProvider, useNavigation } from '../../context';
import type { AppView } from '../../types';
import { BackButton } from './BackButton';

/** Seats the button on `views`, walked in order, so the stack is real. */
function Harness({ views }: { views: AppView[] }) {
  const { view, canGoBack, navigate } = useNavigation();
  useEffect(() => {
    views.forEach((v, i) => navigate(v, i === 0 ? 'replace' : 'push'));
  }, [navigate, views]);
  return (
    <>
      <span data-testid="view">{view.kind}</span>
      <span data-testid="can-go-back">{String(canGoBack)}</span>
      <BackButton />
    </>
  );
}

function mount(views: AppView[]) {
  return render(<Harness views={views} />, { wrapper: NavigationProvider });
}

const settings: AppView = { kind: 'settings' };
const editor: AppView = { kind: 'workflow-editor', workflowId: 'wf-1' };

describe('BackButton', () => {
  // Entered from Settings, not from the workflow list — so where the user came
  // from and where the editor *sits* are different answers, and only a pop
  // gives the first. Reached from the list they coincide, which is why a test
  // written that way passes against the hard-coded destination this replaced.
  it('pops the stack rather than navigating to a destination of its own', async () => {
    mount([settings, editor]);
    expect(screen.getByTestId('view')).toHaveTextContent('workflow-editor');

    await userEvent.click(screen.getByTestId('back-button'));

    expect(screen.getByTestId('view')).toHaveTextContent('settings');
  });

  it('names where it leads, so a bare arrow is not a guess', () => {
    mount([settings, editor]);
    expect(screen.getByTestId('back-button')).toHaveAttribute('title', 'Back to settings');
  });

  // A screen reached by deep link has no history at all. Going *up* is the
  // fallback there, and it replaces rather than pushes — a Back that grew the
  // stack is the bug this control exists to end.
  it('goes up to the parent when there is no history, without growing the stack', async () => {
    mount([editor]);
    expect(screen.getByTestId('back-button')).toHaveAttribute('title', 'Back to workflows');

    expect(screen.getByTestId('can-go-back')).toHaveTextContent('false');

    await userEvent.click(screen.getByTestId('back-button'));
    expect(screen.getByTestId('view')).toHaveTextContent('workflows');

    // Still nothing behind us: going up replaced the entry instead of banking
    // the editor behind it, so a second press cannot walk back into the screen
    // the first one just left.
    expect(screen.getByTestId('can-go-back')).toHaveTextContent('false');
  });

  it('is disabled rather than absent at a root, so the header does not reflow', () => {
    mount([{ kind: 'home' }]);
    const button = screen.getByTestId('back-button');
    expect(button).toBeInTheDocument();
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute('title', 'Back');
  });

  it('carries an accessible name, having no visible text of its own', () => {
    mount([settings, editor]);
    expect(screen.getByRole('button', { name: 'Back' })).toBeInTheDocument();
  });
});
