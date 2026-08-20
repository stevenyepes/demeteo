export function formatTokens(tokens: number | null | undefined): string {
  if (tokens == null) return '0';
  if (tokens >= 1_000_000) {
    return `${(tokens / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  }
  if (tokens >= 1_000) {
    return `${(tokens / 1_000).toFixed(1).replace(/\.0$/, '')}k`;
  }
  return tokens.toString();
}

export function formatCost(usd: number | null | undefined): string {
  if (usd == null || !Number.isFinite(usd)) return '$0.00';
  if (usd < 0.005) return '<$0.01';
  if (usd < 1) return `$${usd.toFixed(3)}`;
  if (usd < 100) return `$${usd.toFixed(2)}`;
  return `$${usd.toFixed(0)}`;
}

/** Compact wall-clock duration ("45s", "2m 10s", "1h 5m") from a second
 *  count — the run-surface vocabulary shared by the timeline chips and the
 *  workflow canvas node cards (P2.2). */
export function formatDuration(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs)) return '0s';
  const s = Math.round(secs);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  if (m < 60) return rem > 0 ? `${m}m ${rem}s` : `${m}m`;
  const h = Math.floor(m / 60);
  const remM = m % 60;
  return remM > 0 ? `${h}h ${remM}m` : `${h}h`;
}

/** Coarse "3m ago"-style age for a millisecond timestamp. `now` is a parameter
 *  so a caller that derives several ages at once reads them against one instant
 *  — and so a test can pin it without faking the clock. */
export function relativeTime(ms: number, now: number = Date.now()): string {
  const deltaSec = Math.max(0, Math.floor((now - ms) / 1000));
  if (deltaSec < 60) return 'just now';
  const deltaMin = Math.floor(deltaSec / 60);
  if (deltaMin < 60) return `${deltaMin}m ago`;
  const deltaHr = Math.floor(deltaMin / 60);
  if (deltaHr < 24) return `${deltaHr}h ago`;
  return `${Math.floor(deltaHr / 24)}d ago`;
}

export function cacheSavingsUsd(
  cacheReadTokens: number | null | undefined,
  costPerMillionInputUsd: number | null | undefined,
): number | null {
  if (
    cacheReadTokens == null ||
    cacheReadTokens <= 0 ||
    costPerMillionInputUsd == null ||
    costPerMillionInputUsd <= 0
  ) {
    return null;
  }
  // Anthropic's prompt-cache read price is ~10% of base input. We
  // charge the *difference* as the saving — without caching the
  // user would have paid `cache_read × price`; with caching they
  // paid `cache_read × (price × 0.1)`. Savings = cache_read × price
  // × 0.9.
  return (cacheReadTokens / 1_000_000) * costPerMillionInputUsd * 0.9;
}
