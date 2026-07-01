/**
 * Pure client-side vision-capability inference for a model string.
 *
 * Mirrors `application::agent_probe::model_supports_images_by_name` on the
 * Rust side. The bundled fallback table in `agent_probe::fallback_models`
 * is the authoritative source — this helper exists so the UI can render
 * a soft warning *before* a model has been selected from the dropdown
 * (e.g. when the user types a custom override or when the probe returns
 * a model whose name doesn't appear in the bundled table).
 *
 * **Pessimistic by design.** Any model string that isn't a positive
 * match (after the negative overrides) returns `false` so the UI
 * never silently drops an attached image on a non-vision model.
 *
 * Rules (substring match, case-insensitive):
 *
 *   positive — `gpt-4`, `gemini`, `claude`, `vision`, `opus`,
 *              `sonnet`, `haiku`
 *   negative — `embedding`, `whisper` (overrides positives)
 */
export function modelSupportsImagesByName(
  _agentKind: string,
  model: string,
): boolean {
  const m = (model ?? "").trim().toLowerCase();
  if (m.length === 0) return false;
  if (m.includes("embedding") || m.includes("whisper")) return false;
  const positives = [
    "gpt-4",
    "gemini",
    "claude",
    "vision",
    "opus",
    "sonnet",
    "haiku",
  ];
  return positives.some((needle) => m.includes(needle));
}