import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';

import { type FleetInstance, getFleet } from './fleet';

export type FleetRead = {
  fleet: FleetInstance[] | null;
  failure: { error: unknown } | null;
};

const NOT_READ: FleetRead = { fleet: null, failure: null };

type FleetHeld = FleetRead & { forgetInstance: (instanceId: string) => void };

const FleetContext = createContext<FleetHeld | null>(null);

interface FleetProviderProps {
  /** Changing it re-reads the fleet; the shell passes the shown view's key. */
  readKey: string;
  children: ReactNode;
}

/**
 * One fleet read shared by the rail's needs-you badge and the Fleet view, so a
 * signed-in page asks `/v1/fleet` once rather than once per consumer. A re-read
 * keeps the last answer on screen until the next one lands: a badge that
 * blanked on every navigation would claim the gates had gone.
 */
export function FleetProvider({ readKey, children }: FleetProviderProps) {
  const [read, setRead] = useState<FleetRead>(NOT_READ);

  /** Bumped on every read and on unmount, so only the newest answer lands. */
  const latestRead = useRef(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `readKey` is the trigger, not an input.
  useEffect(() => {
    latestRead.current += 1;
    const current = latestRead.current;
    getFleet().then(
      (fleet) => {
        if (latestRead.current === current) setRead({ fleet, failure: null });
      },
      (error: unknown) => {
        if (latestRead.current === current) setRead((last) => ({ fleet: last.fleet, failure: { error } }));
      },
    );
    return () => {
      latestRead.current += 1;
    };
  }, [readKey]);

  /**
   * Drops an instance the Hub no longer has from the held answer at once. The
   * re-read that the next view's key starts keeps this answer on screen until
   * it lands, and until then the removed instance would still be listed.
   */
  const forgetInstance = useCallback((instanceId: string) => {
    setRead((last) => ({
      ...last,
      fleet: last.fleet === null ? null : last.fleet.filter((instance) => instance.id !== instanceId),
    }));
  }, []);

  const held = useMemo(() => ({ ...read, forgetInstance }), [read, forgetInstance]);

  return <FleetContext.Provider value={held}>{children}</FleetContext.Provider>;
}

/** Outside a `FleetProvider` nothing has been read, which is what it returns. */
export function useFleet(): FleetRead {
  return useContext(FleetContext) ?? NOT_READ;
}

/** Outside a `FleetProvider` there is nothing held to forget, so it does nothing. */
export function useForgetInstance(): (instanceId: string) => void {
  return useContext(FleetContext)?.forgetInstance ?? forgetNothing;
}

function forgetNothing(): void {}
