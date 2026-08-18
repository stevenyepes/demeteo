/**
 * The fold both transports run over a run's event log. The cases that matter
 * are the ones a caller cannot repair afterwards: order, identity when nothing
 * is new, and what survives the cap.
 */
import { describe, expect, it } from 'vitest';

import { EMPTY_RUN_EVENT_FEED, MAX_FEED_EVENTS, mergeRunEventFeed } from './runEventFeed';
import type { RunEvent } from '../types';

const event = (offset: number, over: Partial<RunEvent> = {}): RunEvent => ({
  offset,
  run_id: 'f1',
  kind: 'step_progress',
  payload_json: null,
  created_at: offset,
  ...over,
});

const spawned = (offset: number, stepExecutionId: string, agentKind: string): RunEvent =>
  event(offset, {
    kind: 'agent_spawned',
    payload_json: JSON.stringify({
      step_execution_id: stepExecutionId,
      agent_kind: agentKind,
      effort: 'high',
    }),
  });

const offsets = (n: number, from = 1) => Array.from({ length: n }, (_, i) => event(i + from));

describe('mergeRunEventFeed', () => {
  it('orders by durable offset regardless of delivery order', () => {
    const feed = mergeRunEventFeed(EMPTY_RUN_EVENT_FEED, [event(8), event(4)]);
    expect(mergeRunEventFeed(feed, [event(6)]).events.map((e) => e.offset)).toEqual([4, 6, 8]);
  });

  it('returns the same feed when every row is a duplicate', () => {
    const feed = mergeRunEventFeed(EMPTY_RUN_EVENT_FEED, [event(1), event(2)]);
    expect(mergeRunEventFeed(feed, [event(2), event(1), event(1)])).toBe(feed);
  });

  it('de-dupes within a single batch', () => {
    const feed = mergeRunEventFeed(EMPTY_RUN_EVENT_FEED, [event(3), event(3)]);
    expect(feed.events).toHaveLength(1);
  });

  it('keeps assignments whose row the cap has already dropped', () => {
    const feed = mergeRunEventFeed(EMPTY_RUN_EVENT_FEED, [
      spawned(1, 'se-1', 'hermes'),
      ...offsets(MAX_FEED_EVENTS + 20, 2),
    ]);

    expect(feed.events).toHaveLength(MAX_FEED_EVENTS);
    expect(feed.events.some((e) => e.offset === 1)).toBe(false);
    expect(feed.assignments['se-1']).toMatchObject({ agentKind: 'hermes', offset: 1 });
  });

  it('bounds its de-dupe bookkeeping to the retained window', () => {
    let feed = mergeRunEventFeed(EMPTY_RUN_EVENT_FEED, offsets(MAX_FEED_EVENTS));
    for (const e of offsets(200, MAX_FEED_EVENTS + 1)) feed = mergeRunEventFeed(feed, [e]);

    expect(feed.seenOffsets.size).toBe(MAX_FEED_EVENTS);
    // An evicted row is refused by the eviction watermark, not by a set that
    // would have to remember every offset the view has ever seen.
    expect(mergeRunEventFeed(feed, [event(1)])).toBe(feed);
  });
});
