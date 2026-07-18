// Public hook surface for the VSCode-style terminal panel.
//
// The actual hook lives in `src/context/TerminalPanelProvider.tsx` so it
// shares the same `createContext` symbol as the provider. Re-exporting
// from the conventional `src/hooks/` location lets consumers stay
// decoupled from where the context was defined — `useTerminalPanel` is
// the canonical name across the app.
//
// Usage:
//   function MyLaunchButton() {
//     const panel = useTerminalPanel();
//     return <button onClick={() => panel.open({ machineId: 'local', machineLabel: 'local' })}>Open terminal</button>;
//   }

export {
  useTerminalPanel,
} from '../context/TerminalPanelProvider';
export type {
  TerminalPanelOpenInput,
  TerminalPanelContextValue,
} from '../context/TerminalPanelProvider';