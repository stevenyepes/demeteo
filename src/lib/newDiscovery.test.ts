import { describe, expect, it } from 'vitest';

import {
  interviewerMachineOptions,
  nameFieldState,
  noVisionNote,
  suggestedBranchName,
  TITLE_MAX_CHARS,
} from './newDiscovery';
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

describe('nameFieldState', () => {
  it('says nothing under a name that is nowhere near the cap', () => {
    expect(nameFieldState('Ask-the-repo chat').showCounter).toBe(false);
  });

  it('counts down once the name is long enough to be cut off', () => {
    const near = 'n'.repeat(TITLE_MAX_CHARS - 4);
    expect(nameFieldState(near)).toEqual({
      remaining: 4,
      showCounter: true,
      overLimit: false,
    });
  });

  it('refuses a seed that arrived past the cap without a keystroke', () => {
    const pasted = 'n'.repeat(TITLE_MAX_CHARS + 12);
    expect(nameFieldState(pasted)).toEqual({
      remaining: -12,
      showCounter: true,
      overLimit: true,
    });
  });

  it('measures the trimmed name, so trailing space never costs a word', () => {
    const atCap = 'n'.repeat(TITLE_MAX_CHARS);
    expect(nameFieldState(`  ${atCap}  `).overLimit).toBe(false);
  });

  // `String.length` counts UTF-16 units, so an astral character is two of them
  // and a name of them reads as twice its length to anything counting that way.
  it('measures characters, not the units they happen to take', () => {
    const name = '\u{1f9e0}'.repeat(TITLE_MAX_CHARS / 2 + 5);
    expect(name.length).toBeGreaterThan(TITLE_MAX_CHARS);
    expect(nameFieldState(name).overLimit).toBe(false);
  });
});

describe('suggestedBranchName', () => {
  it('lowercases and hyphenates the title, then prefixes it verbatim', () => {
    expect(suggestedBranchName('Ask the repo chat', 'demeteo/features/')).toBe(
      'demeteo/features/ask-the-repo-chat',
    );
  });

  it('collapses runs of punctuation to one hyphen and trims the edges', () => {
    expect(suggestedBranchName('  Fix -- the!! bug??  ', 'demeteo/features/')).toBe(
      'demeteo/features/fix-the-bug',
    );
  });

  // An all-punctuation title has nothing alphanumeric to survive collapsing,
  // so it must fall back rather than produce an empty or prefix-only name.
  it('falls back to discovery for an all-punctuation title', () => {
    expect(suggestedBranchName('!!!???', 'demeteo/features/')).toBe(
      'demeteo/features/discovery',
    );
  });

  it('falls back to discovery for a title with no latin or digit characters', () => {
    expect(suggestedBranchName('日本語のタイトル', 'demeteo/features/')).toBe(
      'demeteo/features/discovery',
    );
  });

  it('caps the slug at 48 characters rather than passing it through whole', () => {
    const longTitle = 'a'.repeat(80);
    const result = suggestedBranchName(longTitle, 'demeteo/features/');
    expect(result).toBe(`demeteo/features/${'a'.repeat(48)}`);
    expect(result.length).toBe('demeteo/features/'.length + 48);
  });

  // Twelve 3-letter words leave a hyphen at slug index 47 — the pre-slice
  // trim never sees it, so the 48-char cut must strip it again afterward.
  it('strips a trailing hyphen left by truncation at the 48-char cap', () => {
    const title = 'aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm';
    expect(suggestedBranchName(title, 'demeteo/features/')).toBe(
      'demeteo/features/aaa-bbb-ccc-ddd-eee-fff-ggg-hhh-iii-jjj-kkk-lll',
    );
  });

  it('uses the bare slug when branchPrefix has not resolved yet', () => {
    expect(suggestedBranchName('Ask the repo chat', '')).toBe('ask-the-repo-chat');
  });
});
