// The composer's half of `docs/TASKS_DISCOVERY.md` "Phase 2b": a file is added
// to the Discovery, not to a turn, so the chip row is whatever the backend
// holds and survives every turn taken after it.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { InterviewComposer } from './InterviewComposer';
import type { AttachedFile } from '../../lib/attachments';

afterEach(cleanup);

const TOPOLOGY: AttachedFile = {
  id: 'at-1',
  name: 'runner-topology.png',
  mime: 'image/png',
  sha256: 'a'.repeat(64),
  size: 2048,
  source_filename: 'runner-topology.png',
};

function renderComposer(props: Partial<React.ComponentProps<typeof InterviewComposer>> = {}) {
  return render(
    <InterviewComposer
      discoveryId="dsc-1"
      agentKind="claude-code"
      model="opus"
      machineId="local"
      attachments={[]}
      awaiting={false}
      pending={false}
      disabled={false}
      value=""
      onChange={() => {}}
      onSend={() => {}}
      onAttachmentsChanged={() => {}}
      inputRef={createRef<HTMLInputElement>()}
      {...props}
    />,
  );
}

describe('InterviewComposer', () => {
  it('shows what the Discovery already holds, with nothing clicked', () => {
    renderComposer({ attachments: [TOPOLOGY] });

    expect(screen.getByText('runner-topology.png')).toBeTruthy();
  });

  it('offers no chip row at all until there is something to put in it', () => {
    renderComposer();

    expect(screen.queryByText('Attachments')).toBeNull();
  });

  it('opens the dropzone from the paperclip', () => {
    renderComposer();

    fireEvent.click(screen.getByTestId('interview-attach'));

    expect(screen.getByText('Attachments')).toBeTruthy();
    expect(screen.getByTestId('interview-attach').getAttribute('aria-expanded')).toBe('true');
  });

  it('sends the turn from the button and from Enter alike', () => {
    const onSend = vi.fn();
    renderComposer({ attachments: [TOPOLOGY], value: 'the runner, then', onSend });

    fireEvent.click(screen.getByTestId('interview-send'));
    fireEvent.keyDown(screen.getByTestId('interview-composer'), { key: 'Enter' });

    expect(onSend).toHaveBeenCalledTimes(2);
  });

  it('takes no turn and no file while the interview is closed', () => {
    renderComposer({ disabled: true, value: 'late thought', attachments: [TOPOLOGY] });

    expect(screen.getByTestId('interview-send').hasAttribute('disabled')).toBe(true);
    expect(screen.getByTestId('interview-attach').hasAttribute('disabled')).toBe(true);
  });
});
