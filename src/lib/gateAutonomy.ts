import type { GateAutonomy } from "../types";

export function isGateAutonomy(v: unknown): v is GateAutonomy {
  return v === "attended" || v === "review" || v === "full";
}
