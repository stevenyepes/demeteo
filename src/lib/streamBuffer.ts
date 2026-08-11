/**
 * Bounded accumulation for the live agent stream.
 *
 * The stream buffer is rendered into a `whitespace-pre-wrap` block inside a
 * fixed-height scroller, so the browser lays out the *whole* text height on
 * every flush — up to once per animation frame while an agent streams. Capping
 * at accumulate time is what keeps that height constant; capping at render time
 * would still hold the megabytes in the state the flush copies.
 *
 * Counted in UTF-16 code units rather than encoded bytes: `TextEncoder` would
 * re-encode the buffer on every append — the exact per-flush cost this module
 * exists to remove — and layout cost tracks characters and lines, not bytes.
 */
export const STREAM_CAP_CHARS = 256 * 1024;

/**
 * Smallest share of the cap a line-boundary cut may leave. Preferring a line
 * boundary unconditionally loses the buffer to a single pathological line: one
 * 1 MB line terminated at the very end has its only newline *after* the whole
 * retained window, so cutting there yields the empty string. Below this floor a
 * mid-line cut is the lesser evil.
 */
const MIN_BOUNDARY_KEEP = 0.5;

/**
 * Append `chunk`, keeping at most `cap` code units of the **tail** — an agent's
 * useful output is what it said last, so overflow is dropped from the front.
 */
export function appendCapped(
  prev: string,
  chunk: string,
  cap: number = STREAM_CAP_CHARS,
): string {
  if (chunk === '') return prev;

  const next = prev + chunk;
  if (next.length <= cap) return next;

  const hardCut = next.length - cap;
  const boundary = next.indexOf('\n', Math.max(0, hardCut - 1));
  if (boundary !== -1 && next.length - boundary - 1 >= cap * MIN_BOUNDARY_KEEP) {
    return next.slice(boundary + 1);
  }
  return next.slice(hardCut);
}

/**
 * Whether that append dropped leading text, so a caller can mark the buffer as
 * a tail rather than presenting it as the agent's full output.
 *
 * Kept out of `appendCapped`'s return type on purpose: a `{ text, truncated }`
 * wrapper allocates once per flush and, being a fresh object every time, is a
 * new prop identity on every frame for a consumer that only ever compares the
 * text — the re-render fan-out the cap exists to shrink. Truncation is sticky
 * per step, but that state belongs to the caller that owns the buffer.
 */
export function wasTruncated(prev: string, chunk: string, next: string): boolean {
  return next.length < prev.length + chunk.length;
}
