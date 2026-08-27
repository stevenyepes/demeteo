/**
 * The pane's own contract for a long transcript: which lines are on screen
 * before anybody presses anything. `lib/rawOutput.test.ts` asserts the split
 * itself; what is here is that the head, the count and the verdict all reach
 * the DOM, and that a short `detail` still renders as the one `<pre>` every
 * other sync state has always shown.
 */
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { SyncRawOutput } from './SyncRawOutput';

const ESC = '\u001b';

const longTranscript = [
  "The merge is committed on this branch and the project's checks failed in it, so it was not pushed.",
  '',
  '$ npm run checks:code',
  'Command failed (exit code: Some(101)): ',
  `${ESC}[1;34m==>${ESC}[0m Toolchain preflight`,
  ...Array.from({ length: 2000 }, (_, i) => `Found ${i + 1} warnings.`),
  'error: could not compile `demeteo-core` (lib) due to 4 previous errors',
].join('\n');

const box = () => screen.getByTestId('sync-raw-error');

describe('SyncRawOutput', () => {
  it('renders a short transcript whole, with no control to press', () => {
    render(<SyncRawOutput text="fatal: could not read from remote repository." />);

    expect(box().tagName).toBe('PRE');
    expect(box()).toHaveTextContent('fatal: could not read from remote repository.');
    expect(screen.queryByTestId('sync-raw-error-toggle')).toBeNull();
    expect(screen.queryByTestId('sync-raw-error-elision')).toBeNull();
  });

  it('shows the command, the count of what it hid, and the verdict', () => {
    render(<SyncRawOutput text={longTranscript} />);

    const text = box().textContent ?? '';
    expect(text).toContain('$ npm run checks:code');
    expect(text).toContain('Command failed (exit code: Some(101))');
    expect(text).toContain('error: could not compile `demeteo-core` (lib) due to 4 previous errors');
    expect(text).not.toContain('Found 1000 warnings.');
    expect(screen.getByTestId('sync-raw-error-elision')).toHaveTextContent(/\d+ lines hidden/);
  });

  it('renders no escape sequence a terminal would have eaten', () => {
    render(<SyncRawOutput text={longTranscript} />);

    expect(box().textContent ?? '').toContain('==> Toolchain preflight');
    expect(box().textContent ?? '').not.toContain(ESC);
  });

  /** Requirement, not decoration: the head is four lines of preamble and the
   *  answer is at the far end, so a box that opens at the top opens on nothing.
   *  jsdom lays nothing out, hence the stubbed height — what is asserted is
   *  that the pane scrolls to whatever the height turns out to be. */
  it('parks a long transcript at its end', () => {
    const height = vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(4096);
    render(<SyncRawOutput text={longTranscript} />);

    expect(box().scrollTop).toBe(4096);
    height.mockRestore();
  });

  it('hands over the whole transcript on a press, and takes it back', async () => {
    render(<SyncRawOutput text={longTranscript} />);
    const toggle = screen.getByTestId('sync-raw-error-toggle');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');

    await userEvent.click(toggle);

    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(box().textContent ?? '').toContain('Found 1000 warnings.');
    expect(screen.queryByTestId('sync-raw-error-elision')).toBeNull();

    await userEvent.click(toggle);

    expect(box().textContent ?? '').not.toContain('Found 1000 warnings.');
  });
});
