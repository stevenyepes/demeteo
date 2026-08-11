import { describe, expect, it } from 'vitest';
import { STREAM_CAP_CHARS, appendCapped, wasTruncated } from './streamBuffer';

function lines(count: number, from = 0): string {
  let out = '';
  for (let i = from; i < from + count; i += 1) out += `line ${i}\n`;
  return out;
}

describe('appendCapped', () => {
  it('appends plainly while the result fits the cap', () => {
    expect(appendCapped('abc', 'def', 100)).toBe('abcdef');
  });

  // The first two assertions are tautological on their own — a string return is
  // compared by value, so no implementation producing the right content can fail
  // them. The third is what makes the empty-chunk branch observable: an empty
  // chunk is not new data, so it must not re-trim a buffer that is already over
  // the cap.
  it('leaves the buffer untouched for an empty chunk', () => {
    const prev = lines(3);
    expect(appendCapped(prev, '', 100)).toBe(prev);
    expect(appendCapped(prev, '')).toBe(prev);

    const over = 'x'.repeat(200);
    expect(appendCapped(over, '', 100)).toBe(over);
  });

  it('keeps the tail and drops from the front', () => {
    const prev = lines(40);
    const chunk = lines(2, 40);
    const capped = appendCapped(prev, chunk, 100);

    expect(capped.length).toBeLessThanOrEqual(100);
    expect(capped.endsWith('line 41\n')).toBe(true);
    expect(capped.startsWith('line 0\n')).toBe(false);
    expect((prev + chunk).endsWith(capped)).toBe(true);
  });

  it('cuts on a line boundary, never mid-line', () => {
    const full = lines(40);
    const capped = appendCapped(full, lines(1, 40), 100);

    expect(capped.split('\n')[0]).toMatch(/^line \d+$/);
  });

  it('hard-cuts when the retained window holds no newline', () => {
    const chunk = 'x'.repeat(500);
    const capped = appendCapped('', chunk, 100);

    expect(capped).toBe('x'.repeat(100));
  });

  it('keeps a usable tail when the only newline is far behind it', () => {
    // One 500-char line terminated at the very end: cutting at that newline
    // would leave the empty string, so the boundary preference has a floor.
    const capped = appendCapped('', `${'x'.repeat(500)}\n`, 100);

    expect(capped.length).toBeGreaterThanOrEqual(50);
    expect(capped.length).toBeLessThanOrEqual(100);
    expect(capped.endsWith('\n')).toBe(true);
  });

  it('caps a single chunk larger than the cap on its own', () => {
    const chunk = lines(200);
    const capped = appendCapped('', chunk, 100);

    expect(capped.length).toBeLessThanOrEqual(100);
    expect(capped.endsWith('line 199\n')).toBe(true);
    expect(chunk.endsWith(capped)).toBe(true);
  });

  it('stays bounded across many appends', () => {
    let buf = '';
    for (let i = 0; i < 500; i += 1) buf = appendCapped(buf, lines(1, i), 100);

    expect(buf.length).toBeLessThanOrEqual(100);
    expect(buf.endsWith('line 499\n')).toBe(true);
  });

  it('defaults to the module cap', () => {
    const buf = appendCapped('', lines(60_000));

    expect(buf.length).toBeLessThanOrEqual(STREAM_CAP_CHARS);
    expect(buf.length).toBeGreaterThan(STREAM_CAP_CHARS / 2);
    expect(STREAM_CAP_CHARS).toBe(256 * 1024);
  });
});

describe('wasTruncated', () => {
  it('is false for an append that fit', () => {
    const prev = lines(3);
    const chunk = lines(1, 3);
    expect(wasTruncated(prev, chunk, appendCapped(prev, chunk, 100))).toBe(false);
  });

  it('is false for an empty chunk', () => {
    const prev = lines(3);
    expect(wasTruncated(prev, '', appendCapped(prev, '', 100))).toBe(false);
  });

  it('is true for an append that dropped leading text', () => {
    const prev = lines(40);
    const chunk = lines(2, 40);
    expect(wasTruncated(prev, chunk, appendCapped(prev, chunk, 100))).toBe(true);
  });
});
