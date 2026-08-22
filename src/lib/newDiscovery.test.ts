import { describe, expect, it } from 'vitest';

import { interviewerMachineOptions, noVisionNote } from './newDiscovery';
import type { Machine } from '../types';

function machine(id: string, name: string): Machine {
  return {
    id,
    name,
    host: `${id}.example`,
    port: 22,
    username: 'demeteo',
    auth_type: 'key',
  };
}

describe('interviewerMachineOptions', () => {
  it('always offers the desktop host, which has no machines row to come from', () => {
    expect(interviewerMachineOptions([], 'local')).toEqual([{ id: 'local', label: 'local' }]);
  });

  it('labels a configured machine by name and falls back to its id', () => {
    const options = interviewerMachineOptions(
      [machine('m1', 'runner-01'), machine('m2', '')],
      'local',
    );

    expect(options).toEqual([
      { id: 'local', label: 'local' },
      { id: 'm1', label: 'runner-01' },
      { id: 'm2', label: 'm2' },
    ]);
  });

  // A select with no option for its own value shows the first one instead, so
  // the user reads a machine they never chose and confirms it by pressing
  // Start.
  it('keeps the project host on the list when nothing is configured for it', () => {
    const options = interviewerMachineOptions([machine('m1', 'runner-01')], 'gone');

    expect(options.map((o) => o.id)).toEqual(['local', 'm1', 'gone']);
  });

  it('does not offer the project host twice', () => {
    const options = interviewerMachineOptions([machine('m1', 'runner-01')], 'm1');

    expect(options.map((o) => o.id)).toEqual(['local', 'm1']);
  });
});

describe('noVisionNote', () => {
  const png = { mime: 'image/png', name: 'runner-topology.png' };
  const doc = { mime: 'text/markdown', name: 'MULTI_CLIENT_RUNNER.md' };

  it('names only the images, since the rest are read either way', () => {
    expect(
      noVisionNote({ model: 'qwen3-coder', readsImages: false, attachments: [doc, png] }),
    ).toEqual({ model: 'qwen3-coder', filenames: ['runner-topology.png'] });
  });

  it('says nothing when the model reads images', () => {
    expect(noVisionNote({ model: 'opus', readsImages: true, attachments: [png] })).toBeNull();
  });

  it('says nothing when no image was attached', () => {
    expect(
      noVisionNote({ model: 'qwen3-coder', readsImages: false, attachments: [doc] }),
    ).toBeNull();
  });

  it('matches an uppercase mime, which a picker is free to hand it', () => {
    expect(
      noVisionNote({
        model: 'qwen3-coder',
        readsImages: false,
        attachments: [{ mime: 'IMAGE/PNG', name: 'shot.png' }],
      }),
    ).toEqual({ model: 'qwen3-coder', filenames: ['shot.png'] });
  });

  it('names the unset model rather than an empty sentence', () => {
    expect(noVisionNote({ model: '  ', readsImages: false, attachments: [png] })).toEqual({
      model: '(unset)',
      filenames: ['runner-topology.png'],
    });
  });
});
