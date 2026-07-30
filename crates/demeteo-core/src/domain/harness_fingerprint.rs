//! Whether two harness failures are the *same* failure.
//!
//! The C6 persistence gate rests on one comparison: this attempt's failing
//! output against the previous attempt's, normalized so that volatile spans —
//! the per-run worktree path, a formatted timestamp, a long id — cannot make
//! the same failure look new. Getting it wrong in either direction is
//! expensive, and it has been wrong in both: under-normalizing meant the
//! classifier was never consulted at all on npm projects, and over-normalizing
//! would collapse two genuinely different regressions into one.
//!
//! It is a decision about the *harness*, not about the verifier turn, which is
//! why it sits beside [`crate::domain::harness_baseline`] and
//! [`crate::domain::harness_delta`] — the baseline fingerprints its runs
//! through this same function so HB2c compares two strings built the same way.
//! Synchronous and total, like everything in `domain/`: it takes the output the
//! adapter captured and returns a string.

/// Whether a reproduced-unchanged failure should be handed to the triage
/// classifier: only when this attempt's fingerprint exactly matches the prior
/// attempt's persisted one. A first failure (`None`) or a *changed* fingerprint
/// is ongoing progress — no triage (C6.2).
pub fn should_triage(prior_fingerprint: Option<&str>, current_fingerprint: &str) -> bool {
    prior_fingerprint == Some(current_fingerprint)
}

/// Normalize a failing harness/prepare output into a fingerprint that is
/// stable across retries of the *same* failure while still differing for a
/// genuinely different one (C6.2). Conservative: mask only known-volatile
/// spans — the absolute worktree path (which carries the per-run subtask id),
/// formatted date-times, and long numeric runs (epoch/ids of ≥6 digits) — and
/// nothing else, so two runs of the same missing-lib failure
/// fingerprint-**match** while a different regression error still differs.
///
/// The gate is only a cheap pre-filter: a false match costs at most one triage
/// call (the agent still makes the real regression/environment call), so this
/// leans toward *matching* volatile-only differences rather than risk missing
/// a genuine reproduction.
///
/// # Why a digit-run mask alone was not enough
///
/// Masking only ≥6-digit runs silently exempted the single most common
/// volatile span in the ecosystem: a **formatted** timestamp, whose digits
/// come in groups of two to four. Every npm failure ends with
///
/// ```text
/// npm error A complete log of this run can be found in:
/// npm error   /home/dev/.npm/_logs/2026-07-30T17_39_51_520Z-debug-0.log
/// ```
///
/// whose longest digit run is `2026`. That line is not under the worktree
/// path, so neither existing mask touched it, and the fingerprint therefore
/// differed on *every* attempt of the *same* failure. [`should_triage`]
/// requires equality, so on any npm-based project the C6 classifier could
/// never be consulted at all — feature `f-1d0209a0e43d5b67` burned seven
/// validate attempts and ~20 implement attempts on a missing npm script that
/// no source edit could ever fix, and the runner's `step_attempts` rows show
/// `verdict.redirect` seven times and `environment` never once.
///
/// So [`mask_timestamps`] now folds the ISO-8601-ish date-time shape (both the
/// `:` form a log line prints and the `_` form npm substitutes for a
/// filesystem-safe filename), and [`mask_debug_log_index`] folds the trailing
/// `-debug-<n>` sequence of that same filename. Neither shape can carry build
/// signal, so masking them cannot collapse two genuinely different compiler
/// errors or assertion failures — which is the one thing this function must
/// never do, since a false *match* only spends a triage call while a wrong
/// escalation terminates a real regression.
pub fn normalize_failure_fingerprint(output: &str, wt_path: &str) -> String {
    let mut s = output.to_string();
    if !wt_path.is_empty() {
        s = s.replace(wt_path, "<WT>");
    }
    // Timestamps first: the basic ISO form (`20260730T173951Z`) would
    // otherwise be eaten piecemeal by the digit-run mask below.
    let masked = mask_long_digit_runs(&mask_debug_log_index(&mask_timestamps(&s)));
    // Drop trailing whitespace per line so cosmetic reflow doesn't perturb the
    // fingerprint, but keep line structure (don't collapse everything).
    masked
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace every ISO-8601-ish date-time span with `<TS>`.
///
/// Deliberately anchored on a **full** `YYYY-MM-DD` date immediately followed
/// by a time: a bare date, a bare clock time, or a version triple stays intact,
/// so the mask cannot reach anything a build error is made of. Within that
/// anchor it is permissive about the punctuation, because the same instant is
/// written several ways by the tools whose output lands here:
///
/// ```text
/// 2026-07-30T17:39:51.520Z     an ISO log line
/// 2026-07-30T17_39_51_520Z     the same, made filesystem-safe by npm
/// 2026-07-30 17:39:51          a plain log line
/// 2026-07-30T17:39:51+02:00    with a zone offset
/// ```
fn mask_timestamps(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        // Never start a match mid-number: `12026-07-30T…` is not a timestamp,
        // and folding its tail would drop a digit the fingerprint should keep.
        let after_digit = i > 0 && chars[i - 1].is_ascii_digit();
        match (!after_digit).then(|| timestamp_len(&chars[i..])).flatten() {
            Some(n) => {
                out.push_str("<TS>");
                i += n;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// Length in `char`s of the date-time span starting at `c[0]`, or `None` when
/// there isn't one. Split out from [`mask_timestamps`] so the shape is one
/// readable sequence of "must have" steps rather than a nest inside a scanner.
fn timestamp_len(c: &[char]) -> Option<usize> {
    fn digits(c: &[char], at: usize, n: usize) -> bool {
        c.len() >= at + n && c[at..at + n].iter().all(|d| d.is_ascii_digit())
    }

    // YYYY-MM-DD — the anchor. Without it nothing below is even attempted.
    if !(digits(c, 0, 4)
        && c.get(4) == Some(&'-')
        && digits(c, 5, 2)
        && c.get(7) == Some(&'-')
        && digits(c, 8, 2))
    {
        return None;
    }
    let mut i = 10;

    // The date/time separator: `T` in ISO, a space in a plain log line, `_` in
    // a filename that could not use either.
    if !matches!(c.get(i), Some('T' | 't' | ' ' | '_')) {
        return None;
    }
    i += 1;

    // HH?MM?SS, with one separator used consistently.
    if !digits(c, i, 2) {
        return None;
    }
    i += 2;
    let sep = match c.get(i) {
        Some(&s @ (':' | '_' | '-')) => s,
        _ => return None,
    };
    i += 1;
    if !(digits(c, i, 2) && c.get(i + 2) == Some(&sep) && digits(c, i + 3, 2)) {
        return None;
    }
    i += 5;

    // Optional fractional seconds. npm's filename separates them with `_`,
    // which is why this is not simply a `.`.
    if matches!(c.get(i), Some('.' | ',' | '_')) && digits(c, i + 1, 1) {
        i += 1;
        while digits(c, i, 1) {
            i += 1;
        }
    }

    // Optional zone: `Z`, `+HH:MM`, or `+HHMM`.
    if matches!(c.get(i), Some('Z' | 'z')) {
        i += 1;
    } else if matches!(c.get(i), Some('+' | '-')) && digits(c, i + 1, 2) {
        i += 3;
        if c.get(i) == Some(&':') && digits(c, i + 1, 2) {
            i += 3;
        } else if digits(c, i, 2) {
            i += 2;
        }
    }

    Some(i)
}

/// Fold the trailing sequence number of an npm debug log filename
/// (`…-debug-0.log`, `…-debug-1.log`) to `-debug-<N>.log`.
///
/// npm bumps it whenever two runs land on the same millisecond, so it is
/// volatile for the same reason the timestamp beside it is — and a single
/// digit is far below [`mask_long_digit_runs`]'s threshold, deliberately, since
/// short numbers elsewhere in a build log are exactly the line numbers and exit
/// codes the fingerprint must keep. Hence a shape-specific rule rather than a
/// looser digit mask.
fn mask_debug_log_index(s: &str) -> String {
    const MARKER: &str = "-debug-";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(MARKER) {
        let (head, tail) = rest.split_at(i + MARKER.len());
        out.push_str(head);
        // ASCII digits only, so the byte length is also the char length and
        // slicing `tail` at it stays on a boundary.
        let n = tail.chars().take_while(char::is_ascii_digit).count();
        if n > 0 && tail[n..].starts_with(".log") {
            out.push_str("<N>");
            rest = &tail[n..];
        } else {
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// Replace every maximal run of ≥6 ASCII digits with `<N>`. Six is above
/// typical line numbers / exit codes / version components (which we keep) and
/// at or below epoch seconds (10) / epoch millis (13) / long run-ids, which are
/// the volatile spans we want to mask.
fn mask_long_digit_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush_digit_run(&mut out, &mut digits);
            out.push(c);
        }
    }
    flush_digit_run(&mut out, &mut digits);
    out
}

fn flush_digit_run(out: &mut String, digits: &mut String) {
    if digits.is_empty() {
        return;
    }
    if digits.len() >= 6 {
        out.push_str("<N>");
    } else {
        out.push_str(digits);
    }
    digits.clear();
}

#[cfg(test)]
#[path = "../../tests/domain/harness_fingerprint.rs"]
mod tests;
