import { describe, expect, it } from 'vitest';

import type { Machine } from '../types';
import { draftPlacement, placementHost, placementLabel } from './ticketPlacement';

const box: Machine = { id: 'machine-box', name: 'box', host: 'box.lan', port: 22, username: 'dev', auth_type: 'key' };

describe('placementLabel', () => {
  it('names local and a detached machine by its name', () => {
    expect(placementLabel({ kind: 'local' }, [box])).toBe('Local');
    expect(placementLabel({ kind: 'detached', machine_id: box.id }, [box])).toBe('Detached · box');
  });

  it('falls back to the id for a machine no longer configured', () => {
    expect(placementLabel({ kind: 'detached', machine_id: 'machine-gone' }, [box])).toBe(
      'Detached · machine-gone',
    );
  });
});

describe('draftPlacement', () => {
  const detachedDefault = { kind: 'detached', machine_id: box.id } as const;

  it('reads Default as the Discovery default', () => {
    expect(draftPlacement('', detachedDefault)).toEqual(detachedDefault);
    expect(draftPlacement('', { kind: 'local' })).toEqual({ kind: 'local' });
  });

  it('reads an explicit local as Local, even over a detached default', () => {
    expect(draftPlacement('local', detachedDefault)).toEqual({ kind: 'local' });
  });

  it('reads any other id as detached on it', () => {
    expect(draftPlacement('machine-gpu', { kind: 'local' })).toEqual({
      kind: 'detached',
      machine_id: 'machine-gpu',
    });
  });
});

describe('placementHost', () => {
  it('runs Local on the local host and detached on its machine', () => {
    expect(placementHost({ kind: 'local' }, 'build-host')).toBe('build-host');
    expect(placementHost({ kind: 'detached', machine_id: box.id }, 'build-host')).toBe(box.id);
  });
});
