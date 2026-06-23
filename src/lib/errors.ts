import type { AppError } from "../types";

/**
 * Normalize any value caught from a `try/catch` (or rejected promise) into a
 * human-readable string for display. Handles four shapes:
 *
 *  1. `AppError` — the discriminated union returned by the Rust backend.
 *     Falls back to the `message` field.
 *  2. `Error` — JavaScript native errors from the frontend itself.
 *  3. `string` — legacy `Result<T, String>` paths, still used by some
 *     pre-migration commands.
 *  4. Anything else — `String(value)` as a last resort.
 *
 * Use this at every catch site that displays a user-facing message. Do NOT
 * use `String(err)` directly — that prints `[object Object]` for `AppError`
 * objects.
 */
export function formatError(err: unknown): string {
  if (err == null) return "Unknown error";
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message || err.name;
  if (typeof err === "object") {
    const maybe = err as { message?: unknown; kind?: unknown };
    if (typeof maybe.message === "string" && maybe.message.length > 0) {
      return maybe.message;
    }
  }
  return String(err);
}

/**
 * Type guard: returns the error coerced to `AppError | null`. The Rust
 * backend always serializes errors as the tagged-union shape, so this is
 * safe to use immediately after an `await invoke(...)` rejection.
 */
export function asAppError(err: unknown): AppError | null {
  if (err == null || typeof err !== "object") return null;
  const candidate = err as { kind?: unknown; message?: unknown };
  if (typeof candidate.kind === "string" && typeof candidate.message === "string") {
    return { kind: candidate.kind as AppError["kind"], message: candidate.message };
  }
  return null;
}
