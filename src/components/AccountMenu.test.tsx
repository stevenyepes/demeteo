// Behaviour tests for the account control (spec AC4).
//
// The outside-click path uses `fireEvent.mouseDown`, not `userEvent.click`: the
// listener is a `mousedown` handler (the `NotificationBell` precedent), so a
// synthetic click alone would never reach it and the case would pass vacuously.

import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AccountMenu } from './AccountMenu';
import type { Provider } from '../types';

const provider: Provider = {
  id: 'p1',
  type: 'github',
  name: 'GitHub',
  host: 'github.com',
  pat: '',
  username: 'octocat',
  avatarUrl: 'https://example.invalid/octocat.png',
};

// `AccountMenuProps.onNavigateSettings` is nullary, so the `{ kind: 'settings' }`
// payload AC4 names lives at the call site. The double reproduces that call site
// rather than asserting the component invents the view object.
function navigateDouble() {
  const navigate = vi.fn<(view: { kind: string }) => void>();
  const onNavigateSettings = vi.fn(() => navigate({ kind: 'settings' }));
  return { navigate, onNavigateSettings };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('AccountMenu', () => {
  it('exposes a menu trigger whose aria-expanded tracks the open state', async () => {
    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);

    const trigger = screen.getByTestId('topbar-account-trigger');
    expect(trigger).toHaveAttribute('aria-haspopup', 'menu');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');

    await userEvent.click(trigger);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
  });

  it('reveals the menu on click and routes its Settings item to the settings view', async () => {
    const { navigate, onNavigateSettings } = navigateDouble();
    render(
      <AccountMenu connectedProvider={provider} onNavigateSettings={onNavigateSettings} />,
    );

    expect(screen.queryByTestId('topbar-account-menu')).toBeNull();

    await userEvent.click(screen.getByTestId('topbar-account-trigger'));

    const menu = screen.getByTestId('topbar-account-menu');
    expect(menu).toBeInTheDocument();

    await userEvent.click(screen.getByRole('menuitem', { name: 'Settings' }));

    expect(onNavigateSettings).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith({ kind: 'settings' });
  });

  // Activating an item unmounts the element holding focus, so without an explicit
  // restore `document.activeElement` falls back to `<body>` and the user's next Tab
  // starts from the top of the view they just navigated to. Focus is restored
  // *before* `onNavigateSettings`, so a settings view that claims focus itself wins.
  it('returns focus to the trigger when the Settings item is activated', async () => {
    const { onNavigateSettings } = navigateDouble();
    render(
      <AccountMenu connectedProvider={provider} onNavigateSettings={onNavigateSettings} />,
    );

    const trigger = screen.getByTestId('topbar-account-trigger');
    await userEvent.tab();
    expect(trigger).toHaveFocus();

    await userEvent.keyboard('{Enter}');
    expect(screen.getByRole('menuitem', { name: 'Settings' })).toHaveFocus();

    await userEvent.keyboard('{Enter}');

    expect(onNavigateSettings).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('topbar-account-menu')).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it('carries the provider identity line and nothing but Settings', async () => {
    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);

    await userEvent.click(screen.getByTestId('topbar-account-trigger'));

    const menu = screen.getByTestId('topbar-account-menu');
    expect(menu).toHaveTextContent('octocat');
    expect(screen.getAllByRole('menuitem')).toHaveLength(1);
  });

  it('closes on Escape and returns focus to the trigger', async () => {
    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);

    const trigger = screen.getByTestId('topbar-account-trigger');
    await userEvent.click(trigger);
    expect(screen.getByTestId('topbar-account-menu')).toBeInTheDocument();

    // Focus has to leave the trigger first, or `toHaveFocus` below passes on the
    // click alone and asserts nothing about restoration. Opening moves it into
    // the menu, which is what earns the `menu` role.
    expect(screen.getByRole('menuitem', { name: 'Settings' })).toHaveFocus();

    await userEvent.keyboard('{Escape}');

    expect(screen.queryByTestId('topbar-account-menu')).toBeNull();
    expect(trigger).toHaveFocus();
  });

  // The path a container-scoped `onKeyDown` cannot see: clicking the identity
  // line — a non-focusable `<div>` inside the menu — blurs to `<body>`, which the
  // outside-mousedown listener ignores (the target is inside the container) and
  // which is not a descendant of the container either, so a React handler on it
  // never runs and the menu is stuck open. WKWebView reaches the same state
  // without any click: Safari does not focus a `<button>` on mousedown.
  it('closes on Escape after focus has blurred to the document body', async () => {
    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);

    const trigger = screen.getByTestId('topbar-account-trigger');
    await userEvent.click(trigger);

    const menu = screen.getByTestId('topbar-account-menu');
    fireEvent.mouseDown(screen.getByTestId('topbar-account-identity'));
    (document.activeElement as HTMLElement | null)?.blur();

    expect(menu).toBeInTheDocument();
    expect(document.activeElement).toBe(document.body);

    await userEvent.keyboard('{Escape}');

    expect(screen.queryByTestId('topbar-account-menu')).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it('closes on a mousedown outside the control', async () => {
    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);

    await userEvent.click(screen.getByTestId('topbar-account-trigger'));
    expect(screen.getByTestId('topbar-account-menu')).toBeInTheDocument();

    fireEvent.mouseDown(document.body);

    expect(screen.queryByTestId('topbar-account-menu')).toBeNull();
    expect(screen.getByTestId('topbar-account-trigger')).toHaveAttribute('aria-expanded', 'false');
  });

  it('installs the outside-click listener only while the menu is open', async () => {
    const addSpy = vi.spyOn(document, 'addEventListener');
    const removeSpy = vi.spyOn(document, 'removeEventListener');

    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);
    const mousedownAdds = () =>
      addSpy.mock.calls.filter(([type]) => type === 'mousedown').length;

    expect(mousedownAdds()).toBe(0);

    await userEvent.click(screen.getByTestId('topbar-account-trigger'));
    expect(mousedownAdds()).toBe(1);

    await userEvent.keyboard('{Escape}');
    expect(removeSpy.mock.calls.filter(([type]) => type === 'mousedown')).toHaveLength(1);
  });

  it('installs the Escape listener only while the menu is open', async () => {
    const addSpy = vi.spyOn(document, 'addEventListener');
    const removeSpy = vi.spyOn(document, 'removeEventListener');

    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);
    const keydownCalls = (spy: typeof addSpy) =>
      spy.mock.calls.filter(([type]) => type === 'keydown').length;

    expect(keydownCalls(addSpy)).toBe(0);

    await userEvent.click(screen.getByTestId('topbar-account-trigger'));
    expect(keydownCalls(addSpy)).toBe(1);
    expect(keydownCalls(removeSpy)).toBe(0);

    await userEvent.keyboard('{Escape}');
    expect(keydownCalls(removeSpy)).toBe(1);
  });

  it('scopes the menu role to the items and links it to the trigger', async () => {
    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);

    const trigger = screen.getByTestId('topbar-account-trigger');
    await userEvent.click(trigger);

    const menu = screen.getByRole('menu');
    expect(menu).not.toContainElement(screen.getByTestId('topbar-account-identity'));
    expect(trigger).toHaveAttribute('aria-controls', menu.id);
    expect(menu.id).toBeTruthy();
  });

  it('renders the gradient fallback and still names the trigger without a provider', () => {
    render(<AccountMenu connectedProvider={null} onNavigateSettings={() => {}} />);

    expect(screen.getByTestId('topbar-account-avatar-fallback')).toBeInTheDocument();
    expect(screen.queryByRole('img')).toBeNull();

    const name = screen.getByTestId('topbar-account-trigger').getAttribute('aria-label');
    expect(name).toBeTruthy();
  });

  it("renders the provider's avatar with the username as alt text", () => {
    render(<AccountMenu connectedProvider={provider} onNavigateSettings={() => {}} />);

    const avatar = screen.getByAltText('octocat');
    expect(avatar).toHaveAttribute('src', provider.avatarUrl);
    expect(screen.queryByTestId('topbar-account-avatar-fallback')).toBeNull();
  });
});
