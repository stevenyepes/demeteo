import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { openTurn, type LiveTurn, type ToolActivity } from '../../lib/discoveryActivity';
import { TurnActivityStrip } from './TurnActivityStrip';

function strip(turn: LiveTurn): string {
  const { getByTestId } = render(<TurnActivityStrip turn={turn} elapsedMs={0} />);
  return getByTestId('turn-activity').textContent ?? '';
}

const reading: ToolActivity = {
  id: 'call-1',
  kind: 'read',
  target: 'crates/demeteo-core/src/paths.rs',
  done: false,
  failed: false,
};

describe('a turn that has emitted nothing', () => {
  it('says it is preparing while the backend is still setting it up', () => {
    expect(strip(openTurn(Date.now(), 'setting_up'))).toContain('Preparing the turn');
  });

  it('says it is thinking once the agent has it', () => {
    expect(strip(openTurn(Date.now(), 'working'))).toContain('Thinking');
  });
});

describe('a turn that has emitted something', () => {
  it('names the call rather than the phase it was last told about', () => {
    const turn: LiveTurn = { ...openTurn(Date.now(), 'setting_up'), current: reading };
    const shown = strip(turn);
    expect(shown).toContain('Reading');
    expect(shown).not.toContain('Preparing');
  });
});
