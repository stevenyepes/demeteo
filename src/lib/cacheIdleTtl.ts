/** `DEFAULT_CACHE_IDLE_TTL_DAYS` in `crates/demeteo-core/src/domain/cache_release.rs`. */
export const DEFAULT_CACHE_IDLE_TTL_DAYS = 14;

/** The column is a Rust `u32`; a larger number fails the whole settings save. */
const MAX_DAYS = 4_294_967_295;

export type ParsedCacheIdleTtl = { ok: true; days: number | null } | { ok: false; message: string };

/** Blank is `null` (the engine default); `0` is a real value, "never release on idleness". */
export function parseCacheIdleTtlDays(text: string): ParsedCacheIdleTtl {
  const trimmed = text.trim();
  if (trimmed === "") return { ok: true, days: null };
  if (!/^\d+$/.test(trimmed)) {
    return { ok: false, message: "Cache idle days must be a whole number of days, 0 or more." };
  }
  const days = Number(trimmed);
  if (days > MAX_DAYS) {
    return { ok: false, message: `Cache idle days cannot exceed ${MAX_DAYS}.` };
  }
  return { ok: true, days };
}
