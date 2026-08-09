import { TimeoutField } from 'demeteo';

/** The canonical use — a bounded numeric setting with its unit and hint. */
export const Default = () => (
  <div className="w-full max-w-sm">
    <TimeoutField
      label="Step timeout"
      hint="How long a single Step may run before the orchestrator gives up on it."
      value={1800}
      onChange={() => {}}
    />
  </div>
);

/** Several stacked, the way the settings panel presents them. */
export const Stacked = () => (
  <div className="w-full max-w-sm flex flex-col gap-3">
    <TimeoutField
      label="Agent turn timeout"
      hint="Per-invocation ceiling for one agent turn."
      value={600}
      onChange={() => {}}
    />
    <TimeoutField
      label="Gate reminder"
      hint="How long a Gate waits before it notifies you again."
      value={3600}
      onChange={() => {}}
    />
    <TimeoutField
      label="SSH probe timeout"
      hint="Connection probe budget before a machine is marked unreachable."
      value={30}
      onChange={() => {}}
    />
  </div>
);

/** At the floor — the input clamps to a minimum of 10 seconds. */
export const AtTheMinimum = () => (
  <div className="w-full max-w-sm">
    <TimeoutField
      label="Poll interval"
      hint="Clamped to 10 seconds; the stepper moves in 30-second increments."
      value={10}
      onChange={() => {}}
    />
  </div>
);
