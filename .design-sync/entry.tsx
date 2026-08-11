// Design-system entry for claude.ai/design.
//
// Demeteo ships as a Tauri application, not a component library, so there is
// no published `dist/` entry for the converter to bundle. This barrel is the
// design system's public surface: the presentational primitives under
// `src/components/ui/` that carry the visual language (glassmorphism panels,
// the violet/cyan/emerald/ruby status vocabulary) without reaching for a
// Tauri command. Anything that calls `invoke()` cannot render in the design
// agent's browser and is deliberately absent.
//
// `--entry` also anchors the converter's package resolution here: it walks up
// to the nearest named package.json, which is the repo root. Without it the
// converter looks for `node_modules/demeteo/` and dies.

export * from '../src/components/ui/ActivityIndicator';
export * from '../src/components/ui/AgentBadge';
export * from '../src/components/ui/FieldLabel';
export * from '../src/components/ui/HarnessModelPicker';
export * from '../src/components/ui/MachineDot';
export * from '../src/components/ui/Modal';
export * from '../src/components/ui/OverlayPortal';
export * from '../src/components/ui/PhaseBadge';
export * from '../src/components/ui/RailNavItem';
export * from '../src/components/ui/ScrollArea';
export * from '../src/components/ui/SectionCard';
export * from '../src/components/ui/StatusBadge';
export * from '../src/components/ui/TabBar';
export * from '../src/components/ui/TimeoutField';

export * from '../src/components/ui/CreateZeroStepHeader';
export * from '../src/components/ui/CreateZeroStepFooter';
export * from '../src/components/ui/CreateZeroNameStep';
export * from '../src/components/ui/CreateZeroDescriptionStep';
export * from '../src/components/ui/CreateZeroStrategyStep';
export * from '../src/components/ui/CreateZeroAgentStep';
export * from '../src/components/ui/CreateZeroMachineStep';
export * from '../src/components/ui/CreateZeroProviderStep';
export * from '../src/components/ui/CreateZeroWorkflowStep';
export * from '../src/components/ui/CreateZeroLaunchStep';
export * from '../src/components/ui/CreateZeroBootstrapPanel';
