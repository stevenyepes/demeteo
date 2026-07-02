// Minimal type declarations for react-test-renderer so the
// renderer-based wizard tests compile. The runtime API surface is
// small (create, act, ReactTestInstance) and the wizard tests use
// only those entry points.

declare module 'react-test-renderer' {
  import type { ReactElement } from 'react';

  export interface ReactTestInstance {
    type: string | React.ComponentType<unknown>;
    props: Record<string, unknown>;
    children: Array<ReactTestInstance | string>;
    parent: ReactTestInstance | null;
    findAll(predicate: (n: ReactTestInstance) => boolean): ReactTestInstance[];
    findAllByType(type: string | React.ComponentType<unknown>): ReactTestInstance[];
    findAllByProps(props: Record<string, unknown>): ReactTestInstance[];
  }

  export interface ReactTestRenderer {
    root: ReactTestInstance;
    toJSON(): unknown;
    unmount(): void;
  }

  export function create(element: ReactElement): ReactTestRenderer;
  export function act(callback: () => void | Promise<void>): void;
}