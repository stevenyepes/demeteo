import type { AgentConfigView } from "./machines";

/** The harnesses a picker may offer: installed *and* enabled. A picker offering
 *  a harness the machine does not have fails at spawn instead. */
export function offerableAgentKinds(
  configs: ReadonlyArray<Pick<AgentConfigView, "kind" | "enabled" | "available">>,
): string[] {
  return configs.filter((a) => a.enabled && a.available).map((a) => a.kind);
}
