declare module 'react-test-renderer' {
  import type { ReactElement } from 'react';
  export interface ReactTestInstance {
    type: any;
    props: any;
    children: any[];
    parent: ReactTestInstance | null;
  }
  export interface ReactTestRenderer {
    root: ReactTestInstance;
    toJSON(): any;
    unmount(): void;
    update(nextElement: ReactElement): void;
  }
  export function create(element: ReactElement): ReactTestRenderer;
  export function act(callback: () => void | Promise<void>): Promise<void>;
}
