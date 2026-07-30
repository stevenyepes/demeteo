//! How much of a harness log is allowed to travel inside an agent prompt.
//!
//! Every agent adapter passes the turn's prompt as one **trailing positional
//! argument** (`build_claude_args`, and its codex/opencode counterparts), and
//! Linux caps each individual `execve` argv string at `MAX_ARG_STRLEN` — 32
//! pages, 128 KiB — *independently* of `ARG_MAX` (2 MiB on a typical box). So a
//! prompt does not degrade as it grows: at 131,072 bytes `execve` returns
//! `E2BIG` and the agent process never starts at all.
//!
//! That is not hypothetical. `s-validate` is harness-first — the orchestrator
//! runs the gates itself and pastes their combined stdout+stderr into the single
//! agent turn — and the output was embedded verbatim. One observed run
//! (`f-1785364787070`) reached `s-validate` with a **green** harness after
//! spending 3.8 M tokens on `s-implement`, and then failed four times, on two
//! different harnesses, with `Argument list too long`: `npm run checks` had
//! emitted 212 KB across 2,971 lines. Zero tokens were spent on the step; the
//! whole feature parked because a log was too chatty.
//!
//! The budget lives here rather than in the renderer because it is a policy
//! decision — *how much evidence is enough* — and belongs somewhere a test can
//! reach without standing up a driver.

/// The largest single `execve` argument Linux accepts: `MAX_ARG_STRLEN`, fixed
/// at 32 pages. **Not** `ARG_MAX`, which is 16× larger and never the binding
/// limit for a prompt.
///
/// The other two desktop targets bound the same thing differently — macOS
/// enforces a 1 MiB total across argv+envp with no per-string cap, Windows a
/// ~32 KiB command line for the legacy path — so this constant is the *tightest
/// per-argument* ceiling of the three, and staying under it keeps every target
/// safe. It is used for the budget's justification and for the diagnostic
/// message; nothing branches on the host OS.
pub const ARGV_STRING_LIMIT_BYTES: usize = 32 * 4096;

/// Total bytes of harness output one prompt's Harness Results section may
/// carry, shared out across the gates.
///
/// Shared rather than paid per gate, following `build_failure_reason`: a step
/// with five declared gates must not silently grow the prompt fivefold, which
/// is exactly how a per-gate budget converges back on `E2BIG`.
///
/// 32 KiB is a quarter of [`ARGV_STRING_LIMIT_BYTES`], which leaves the rest of
/// the prompt — template, attached spec, implement summaries, artifact
/// contract, operating boundary, verdict contract — the other 96 KiB. It is
/// also roughly 8k tokens, far more than reading pass/fail counts and verbatim
/// failing tests actually needs.
pub const HARNESS_SECTION_BUDGET_BYTES: usize = 32 * 1024;

/// The least any one gate gets while the [floor](HARNESS_GATE_FLOOR_BYTES) is
/// affordable. A window shorter than this cannot reliably carry a stack trace or
/// a clippy diagnostic, and a gate whose evidence is unreadable is worse than
/// one that is honestly absent.
pub const HARNESS_GATE_FLOOR_BYTES: usize = 8 * 1024;

/// What the section may reach when the floor is paid to every gate — never
/// exceeded, even at the floor's expense.
///
/// A floor alone is not enough. `build_failure_reason`'s shared-budget-plus-floor
/// shape lets the total grow linearly once the share drops under the floor,
/// which is harmless at its 2000/500 sizes and is *the original bug again* at
/// these: sixteen gates paying an 8 KiB floor is 128 KiB, and `E2BIG`. So past
/// the point where every gate can afford the floor, the ceiling wins and gates
/// get less than it — an unreadably short window is a bad outcome, a spawn that
/// never happens is a worse one.
///
/// 48 KiB is six gates at the floor, which covers every realistic step (a
/// `prepare` plus the declared harnesses) without the ceiling ever binding.
pub const HARNESS_SECTION_CEILING_BYTES: usize = 48 * 1024;

/// Each gate's share of [`HARNESS_SECTION_BUDGET_BYTES`] — floored at
/// [`HARNESS_GATE_FLOOR_BYTES`], then capped so the section as a whole cannot
/// pass [`HARNESS_SECTION_CEILING_BYTES`] however many gates there are.
pub fn per_gate_budget(gate_count: usize) -> usize {
    let gates = gate_count.max(1);
    (HARNESS_SECTION_BUDGET_BYTES / gates)
        .max(HARNESS_GATE_FLOOR_BYTES)
        .min(HARNESS_SECTION_CEILING_BYTES / gates)
}

/// Clamp one harness log to `budget` bytes, keeping its head and its tail.
///
/// Returns `body` unchanged when it already fits — the overwhelmingly common
/// case, and the one every existing prompt expectation was written against.
///
/// **Head *and* tail, not just the tail.** The tail carries the verdict (the
/// failing assertion, the clippy summary, `test result: FAILED`) and the head
/// carries what the run was (which stage started, which toolchain, the
/// `RUN v3.2.7 <worktree>` banner that says the gate ran *here*). A tail-only
/// window loses the second, and an agent that cannot tell which worktree the
/// evidence came from cannot tell green-for-the-wrong-reason from green. The
/// 1:3 split reflects which end is load-bearing more often.
///
/// The elision banner is not decoration. The surrounding prompt calls this
/// output *authoritative* and forbids re-running the suite, so an agent handed
/// a silently-shortened log would read partial counts as totals and report "5
/// tests passed" for a 3,000-test suite. Naming the omission — and naming
/// `unprovable` as the exit when the missing middle held the evidence — is what
/// keeps the truncation honest.
pub fn window_harness_log(body: &str, budget: usize) -> String {
    if body.len() <= budget {
        return body.to_string();
    }

    let head_budget = budget / 4;
    let tail_budget = budget - head_budget;

    // `budget < body.len()` here, so `head_end < tail_start` strictly: both
    // helpers only ever move *inward*, and the two budgets sum to less than the
    // body. No empty-window degenerate case to guard.
    let head_end = line_start_at_or_before(body, head_budget);
    let tail_start = line_start_at_or_after(body, body.len() - tail_budget);

    let omitted = &body[head_end..tail_start];
    let omitted_lines = omitted.lines().count();

    format!(
        "{head}\
         [… {lines} lines / {kib} KiB omitted from the middle of this log …]\n\
         [The head and tail are shown; the middle is gone. Counts visible here are \
         therefore NOT totals — do not report a pass/fail count you had to infer \
         from this window. If the evidence a criterion needs fell in the omitted \
         middle, record that criterion as unprovable rather than guessing at it.]\n\
         {tail}",
        head = &body[..head_end],
        lines = omitted_lines,
        kib = omitted.len().div_ceil(1024),
        tail = &body[tail_start..],
    )
}

/// The diagnostic for a spawn that died on `E2BIG`.
///
/// The raw `Argument list too long (os error 7)` is classified as an
/// environmental failure and shown to the user as one, which sends them looking
/// at their machine for a defect that is entirely ours: the prompt we built was
/// too big to hand to `execve`. This says so, with the number, so the next
/// occurrence is diagnosable from the message alone.
pub fn argv_too_long_message(binary: &str, prompt_bytes: usize) -> String {
    format!(
        "cannot spawn {binary}: this turn's prompt is {prompt_bytes} bytes, over the \
         {limit}-byte ceiling the OS puts on a single command-line argument, so the \
         process was never started. Nothing about the machine is wrong — the prompt \
         itself is too large. The usual cause is an artifact or harness log embedded \
         verbatim; harness output is budgeted \
         (domain::prompt_budget::HARNESS_SECTION_BUDGET_BYTES), so look at what else \
         this step attaches.",
        binary = binary,
        prompt_bytes = prompt_bytes,
        limit = ARGV_STRING_LIMIT_BYTES,
    )
}

/// The start of the last line that begins at or before `at`, falling back to a
/// char boundary when the prefix holds no line break at all (one enormous
/// line — minified output, a `--no-color`-less progress bar).
fn line_start_at_or_before(s: &str, at: usize) -> usize {
    let at = floor_char_boundary(s, at);
    match s[..at].rfind('\n') {
        Some(nl) => nl + 1,
        None => at,
    }
}

/// The start of the first line that begins at or after `at`, falling back to a
/// char boundary when the suffix holds no line break.
fn line_start_at_or_after(s: &str, at: usize) -> usize {
    let at = ceil_char_boundary(s, at);
    match s[at..].find('\n') {
        Some(nl) => at + nl + 1,
        None => at,
    }
}

/// Largest char boundary `<= at`. `str::floor_char_boundary` is still unstable.
fn floor_char_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary `>= at`.
fn ceil_char_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
#[path = "../../tests/domain/prompt_budget.rs"]
mod tests;
