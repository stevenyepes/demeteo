import { useEffect, useRef, useState } from 'react';

import type { NodeRunStatus } from '../../../src/components/canvas/types';
import { HubApiError } from './api';
import { getRunDetail, type RunDetail } from './runs';
import { sameStatusMap, statusByNode } from './runView';

export type StatusMap = Record<string, NodeRunStatus>;

export type RunDetailLoad =
  | { state: 'loading' }
  | {
      state: 'loaded';
      detail: RunDetail;
      statusByNode: StatusMap;
      /** Epoch ms of the read that answered `detail`. */
      readAt: number;
      /** The latest read after it failed, so `detail` is no longer live. */
      stale: boolean;
    }
  | { state: 'failed'; error: unknown };

const POLL_MS = 5000;

/** One run's mirror, re-read while it is running; shared by the Run page and the Runs preview. */
export function useRunDetail(instanceId: string, featureId: string): RunDetailLoad {
  const [load, setLoad] = useState<RunDetailLoad>({ state: 'loading' });

  /** Bumped on unmount, so a read that settles after it has no page to land on. */
  const latestRead = useRef(0);

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let loaded = false;
    setLoad({ state: 'loading' });
    const poll = () => {
      getRunDetail(instanceId, featureId).then(
        (detail) => {
          if (latestRead.current !== read) return;
          loaded = true;
          setLoad((previous) => settled(previous, detail));
          if (detail.status === 'running') timer = setTimeout(poll, POLL_MS);
        },
        (error: unknown) => {
          if (latestRead.current !== read) return;
          // A poll that fails after a first load keeps the last answer on screen,
          // marked stale, and tries again; a lost session, a withdrawn run or a
          // revoked `read` would only fail the same way forever.
          if (loaded && !endsPolling(error)) {
            setLoad((previous) => (previous.state === 'loaded' ? { ...previous, stale: true } : previous));
            timer = setTimeout(poll, POLL_MS);
          } else {
            setLoad({ state: 'failed', error });
          }
        },
      );
    };
    poll();
    return () => {
      latestRead.current += 1;
      clearTimeout(timer);
    };
  }, [instanceId, featureId]);

  return load;
}

/**
 * The canvas re-seeds its nodes whenever `definition` or `statusByNode`
 * changes identity, so a poll that reports nothing new hands back the objects
 * already on screen rather than equal copies of them.
 */
function settled(previous: RunDetailLoad, detail: RunDetail): RunDetailLoad {
  const next = statusByNode(detail.steps, detail.pendingGates);
  const readAt = Date.now();
  if (previous.state !== 'loaded') return { state: 'loaded', detail, statusByNode: next, readAt, stale: false };
  const held = previous.detail.workflow;
  const workflow =
    held !== null && detail.workflow !== null && JSON.stringify(held) === JSON.stringify(detail.workflow)
      ? held
      : detail.workflow;
  return {
    state: 'loaded',
    detail: { ...detail, workflow },
    statusByNode: sameStatusMap(previous.statusByNode, next) ? previous.statusByNode : next,
    readAt,
    stale: false,
  };
}

export function isNotFound(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'not_found';
}

function isUnauthenticated(error: unknown): boolean {
  return error instanceof HubApiError && error.status === 401;
}

/** The Hub answers a revoked scope with `409`, never `403`. */
export function isScopeNotGranted(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'scope_not_granted';
}

function endsPolling(error: unknown): boolean {
  return isUnauthenticated(error) || isNotFound(error) || isScopeNotGranted(error);
}
