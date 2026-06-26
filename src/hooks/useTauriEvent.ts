import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { DependencyList } from 'react';

/**
 * Subscribe to a Tauri backend event. The handler is stable via ref so the
 * listener is only re-created when `deps` change (default: never).
 */
export function useTauriEvent<T>(
  event: string,
  handler: (payload: T) => void,
  deps: DependencyList = []
) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;

    listen<T>(event, (e) => handlerRef.current(e.payload))
      .then((fn) => {
        if (active) {
          unlisten = fn;
        } else {
          fn();
        }
      })
      .catch((err) => console.error(`[useTauriEvent] failed to subscribe to "${event}"`, err));

    return () => {
      active = false;
      unlisten?.();
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [event, ...deps]);
}
