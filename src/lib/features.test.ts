import { describe, it, expect } from "vitest";
import { isEnvironmentError } from "./features";

describe("isEnvironmentError", () => {
  it("matches the backend's terminal environment failure", () => {
    const msg =
      "Environment not ready — this failure is not something editing the code can fix.\n\n" +
      "The shell could not find `cargo` on PATH (exit 127), so the command never ran.\n";
    expect(isEnvironmentError(msg)).toBe(true);
  });

  it("ignores an ordinary step failure", () => {
    expect(isEnvironmentError("thread 'main' panicked at src/main.rs:4:5")).toBe(false);
  });

  it("treats a missing error message as not an environment failure", () => {
    expect(isEnvironmentError(null)).toBe(false);
    expect(isEnvironmentError(undefined)).toBe(false);
    expect(isEnvironmentError("")).toBe(false);
  });
});
