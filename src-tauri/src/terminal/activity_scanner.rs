//! Phase-2 drain OSC scanner (TERMINAL_ACTIVITY §2b).
//!
//! A small **stateful** scanner that sits between a PTY/SSH read and the
//! broadcast, watching the byte stream for our private, namespaced activity
//! sequence and lifting it back out:
//!
//! ```text
//! ESC ] 777 ; demeteo ; v=1 ; nonce=<hex> ; state=<…> (BEL | ESC \)
//! ```
//!
//! (Wire format verified empirically — see `docs/TERMINAL_ACTIVITY.md`:
//! Claude passes an arbitrary OSC 777 payload through `terminalSequence`
//! **verbatim**, so our namespaced signal rides the PTY intact.)
//!
//! On [`feed`](ActivityScanner::feed) it returns the byte stream with every
//! complete demeteo sequence **stripped** (`forward`) plus the parsed `state`
//! strings for the sequences whose `nonce` matched this launch (`events`).
//! Design constraints (TERMINAL_ACTIVITY §5):
//!
//! * **Engages only on `ESC`** — a run of ordinary output is bulk-copied in one
//!   `extend_from_slice`, never inspected byte-by-byte (the fast path).
//! * **Namespaced** — only `ESC ] 777 ; demeteo ;` engages the buffering path;
//!   every other OSC (0 title, 8 hyperlink, 9, 11 bg, 99, …) flows through
//!   untouched.
//! * **Bounded residual (≤128 B)** — a "sequence" that never terminates within
//!   the bound is treated as *not ours*: its bytes are flushed untouched and
//!   normal scanning resumes (fail-open — output is never lost).
//! * **Chunk-split safe** — a sequence may straddle any number of `feed` calls,
//!   split at any byte (incl. between `ESC` and `]`, mid-params, and between the
//!   `ESC` and `\` of an ST terminator).

/// The namespaced introducer every demeteo activity sequence opens with:
/// `ESC ] 777 ; demeteo ;`. Only a byte stream matching this prefix engages the
/// buffering path; anything else is some other terminal's escape/OSC and passes
/// through verbatim. The introducer contains `ESC` at index 0 **only**, which is
/// what lets a mismatch mid-prefix flush cleanly (see [`ActivityScanner::feed_buffered`]).
const INTRODUCER: &[u8] = b"\x1b]777;demeteo;";

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// The final byte of a String Terminator (`ESC \`). The leading `ESC` is
/// buffered like any other collecting-phase byte; this closes it.
const ST_FINAL: u8 = b'\\';

/// Hard cap on the mid-sequence residual (TERMINAL_ACTIVITY §5). A
/// well-formed sequence is tens of bytes; anything that keeps buffering past
/// this without terminating is not ours, so we fail open and flush it.
const MAX_RESIDUAL_BYTES: usize = 128;

/// The result of feeding one chunk through the scanner.
#[derive(Debug, Default)]
pub struct ScanOutput {
    /// The input stream with every *complete* demeteo sequence removed
    /// (introducer through terminator, inclusive). Everything else — ordinary
    /// output and foreign OSCs — is preserved byte-for-byte and in order.
    pub forward: Vec<u8>,
    /// Parsed `state` strings, one per completed sequence whose `nonce` matched
    /// `expected_nonce`. A stripped-but-rejected sequence (wrong/absent nonce,
    /// missing `state`, unknown `v`) yields no event.
    pub events: Vec<String>,
}

/// Stateful scanner carrying the mid-sequence residual across `feed` calls plus
/// the per-launch nonce that gates event emission.
pub struct ActivityScanner {
    /// Bytes buffered while mid-sequence (empty ⇒ not mid-sequence, the fast
    /// path). Its length + content encodes the phase: shorter than
    /// [`INTRODUCER`] ⇒ still matching the introducer; at/after ⇒ collecting
    /// params until a terminator. Bounded by [`MAX_RESIDUAL_BYTES`].
    residual: Vec<u8>,
    /// The nonce minted for this session's agent launch. Only a sequence
    /// carrying exactly this nonce produces an event (§2b nonce gate — rejects
    /// spoofed or cross-session TTY bleed).
    expected_nonce: String,
}

impl ActivityScanner {
    /// Build a scanner gated to `expected_nonce` (the per-launch nonce embedded
    /// in the agent's reporter hooks).
    pub fn new(expected_nonce: String) -> Self {
        ActivityScanner {
            residual: Vec::new(),
            expected_nonce,
        }
    }

    /// Feed one chunk read off the transport. Returns the stripped
    /// forward stream and any parsed activity events (see [`ScanOutput`]).
    pub fn feed(&mut self, chunk: &[u8]) -> ScanOutput {
        let mut out = ScanOutput::default();
        let mut i = 0;
        while i < chunk.len() {
            if self.residual.is_empty() {
                // Fast path (§1): not mid-sequence. Bulk-copy the run of
                // non-ESC bytes up to the next ESC in a single
                // `extend_from_slice` — the common case never touches the
                // residual and never pushes byte-by-byte.
                let rest = &chunk[i..];
                match rest.iter().position(|&b| b == ESC) {
                    None => {
                        out.forward.extend_from_slice(rest);
                        break;
                    }
                    Some(p) => {
                        out.forward.extend_from_slice(&rest[..p]);
                        // Begin buffering at the ESC; classification happens as
                        // the following bytes arrive (possibly next `feed`).
                        self.residual.push(ESC);
                        i += p + 1;
                    }
                }
            } else {
                self.feed_buffered(chunk[i], &mut out);
                i += 1;
            }
        }
        out
    }

    /// Process one byte while mid-sequence (residual non-empty).
    fn feed_buffered(&mut self, b: u8, out: &mut ScanOutput) {
        let n = self.residual.len();
        if n < INTRODUCER.len() {
            // Introducer-matching phase: keep buffering only while the bytes so
            // far stay a prefix of `ESC]777;demeteo;`. The introducer holds ESC
            // at index 0 only, so a mismatch is never a legitimate
            // continuation — flush what we buffered *untouched* and re-scan `b`
            // from a clean state. This is exactly how foreign OSCs (0/8/9/11/99)
            // and near-namespaces (`777;notify`) pass through verbatim (§2).
            if b == INTRODUCER[n] {
                self.residual.push(b);
            } else {
                out.forward.extend_from_slice(&self.residual);
                self.residual.clear();
                // Re-process `b`: it may itself open a fresh sequence
                // (e.g. `ESC ] ESC ]777;demeteo;…`), so route ESC back into the
                // buffer rather than dropping it into `forward`.
                if b == ESC {
                    self.residual.push(ESC);
                } else {
                    out.forward.push(b);
                }
            }
            return;
        }

        // Collecting phase: the full introducer matched, so we are committed to
        // one of *our* sequences — on completion the whole span is stripped.
        // Buffer params until a terminator (BEL or ST).
        if b == BEL {
            self.complete_sequence(out);
            self.residual.clear();
            return;
        }
        if b == ST_FINAL && self.residual.last() == Some(&ESC) {
            // ST = `ESC \`. The `ESC` was buffered on the previous byte (which
            // may have arrived in an earlier `feed` — a split ST). Drop it and
            // finish.
            self.residual.pop();
            self.complete_sequence(out);
            self.residual.clear();
            return;
        }
        self.residual.push(b);
        if self.residual.len() > MAX_RESIDUAL_BYTES {
            // Never terminated within the bound ⇒ almost certainly not ours.
            // Fail open (§4): flush the buffered bytes untouched and resume
            // normal scanning so no output is ever lost.
            out.forward.extend_from_slice(&self.residual);
            self.residual.clear();
        }
    }

    /// A terminator was seen: parse the buffered params and, if the sequence is
    /// authentic (matching nonce, present `state`, acceptable `v`), push its
    /// `state`. Called with `residual` == `ESC]777;demeteo;` + raw params (no
    /// terminator); the caller clears `residual` afterwards. Never touches
    /// `out.forward` — the sequence is stripped purely by *not* forwarding it.
    fn complete_sequence(&mut self, out: &mut ScanOutput) {
        let params = &self.residual[INTRODUCER.len()..];
        let mut nonce: Option<&[u8]> = None;
        let mut state: Option<&[u8]> = None;
        let mut version: Option<&[u8]> = None;
        // Order-insensitive `key=value` parse over `;`-separated fields.
        for field in params.split(|&b| b == b';') {
            if field.is_empty() {
                continue;
            }
            let Some(eq) = field.iter().position(|&b| b == b'=') else {
                continue;
            };
            let (key, val) = (&field[..eq], &field[eq + 1..]);
            match key {
                b"nonce" => nonce = Some(val),
                b"state" => state = Some(val),
                b"v" => version = Some(val),
                _ => {}
            }
        }
        // Version gate: absent ⇒ accept (default `v=1` semantics).
        // Present-but-unrecognised ⇒ strip silently and emit nothing
        // (forward-compat: a newer sender we can't parse must not surface a
        // bogus state).
        if let Some(v) = version {
            if v != b"1" {
                return;
            }
        }
        // Nonce gate (§8): only a sequence carrying *our* per-launch nonce
        // yields an event. `state` is likewise required. Both a nonce mismatch
        // and a missing `state` still strip the sequence (handled by the caller
        // not forwarding it) but produce no event.
        if let (Some(n), Some(s)) = (nonce, state) {
            if n == self.expected_nonce.as_bytes() {
                if let Ok(state_str) = std::str::from_utf8(s) {
                    out.events.push(state_str.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "a1b2c3d4";

    /// Build a complete demeteo sequence for `state`, terminated with BEL
    /// (`bel = true`) or ST (`ESC \`), using the given nonce.
    fn seq_with_nonce(nonce: &str, state: &str, bel: bool) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"\x1b]777;demeteo;v=1;nonce=");
        v.extend_from_slice(nonce.as_bytes());
        v.extend_from_slice(b";state=");
        v.extend_from_slice(state.as_bytes());
        if bel {
            v.push(BEL);
        } else {
            v.extend_from_slice(b"\x1b\\");
        }
        v
    }

    /// A well-formed sequence for `state` with the matching nonce (BEL-terminated).
    fn good_seq(state: &str) -> Vec<u8> {
        seq_with_nonce(NONCE, state, true)
    }

    /// Feed a series of chunks through one scanner, concatenating the forwarded
    /// bytes and collected events across the calls.
    fn feed_all(scanner: &mut ActivityScanner, chunks: &[&[u8]]) -> (Vec<u8>, Vec<String>) {
        let mut fwd = Vec::new();
        let mut evs = Vec::new();
        for c in chunks {
            let out = scanner.feed(c);
            fwd.extend_from_slice(&out.forward);
            evs.extend(out.events);
        }
        (fwd, evs)
    }

    fn scanner() -> ActivityScanner {
        ActivityScanner::new(NONCE.to_string())
    }

    /// Fast path: a run with no ESC is forwarded verbatim and never buffers.
    #[test]
    fn passes_plain_output_through_untouched() {
        let mut s = scanner();
        let out = s.feed(b"hello, world\r\n$ ls -la\r\n");
        assert_eq!(out.forward, b"hello, world\r\n$ ls -la\r\n");
        assert!(out.events.is_empty());
        assert!(s.residual.is_empty(), "plain output must not buffer");
    }

    /// A full sequence in one chunk (BEL terminator) is stripped entirely and
    /// yields its state.
    #[test]
    fn full_sequence_bel_strips_and_emits() {
        let mut s = scanner();
        let out = s.feed(&good_seq("working"));
        assert!(
            out.forward.is_empty(),
            "a complete sequence must leave no artifact: {:?}",
            out.forward
        );
        assert_eq!(out.events, vec!["working".to_string()]);
    }

    /// A full sequence terminated with ST (`ESC \`) parses the same as BEL.
    #[test]
    fn full_sequence_st_strips_and_emits() {
        let mut s = scanner();
        let out = s.feed(&seq_with_nonce(NONCE, "awaiting_approval", false));
        assert!(out.forward.is_empty(), "ST sequence left an artifact");
        assert_eq!(out.events, vec!["awaiting_approval".to_string()]);
    }

    /// Split across two chunks, at every interesting boundary, still parses and
    /// strips cleanly.
    #[test]
    fn split_across_two_chunks() {
        let full = good_seq("awaiting_input");
        // Split points: after ESC (1), mid-introducer, mid-params, near the end.
        for split in [1usize, 2, 8, 20, full.len() - 1] {
            let mut s = scanner();
            let (fwd, evs) = feed_all(&mut s, &[&full[..split], &full[split..]]);
            assert!(fwd.is_empty(), "split at {split} leaked bytes: {fwd:?}");
            assert_eq!(evs, vec!["awaiting_input".to_string()], "split at {split}");
        }
    }

    /// Split across three chunks, including a cut between the `ESC` and `\` of
    /// the ST terminator.
    #[test]
    fn split_across_three_chunks_including_st() {
        let full = seq_with_nonce(NONCE, "working", false);
        let len = full.len();
        // The ST is the final two bytes `ESC \`; cut right between them so the
        // ESC ends chunk 2 and the `\` opens chunk 3.
        let a = 2; // between `]` and `7`
        let b = len - 1; // between ESC and `\`
        let mut s = scanner();
        let (fwd, evs) = feed_all(&mut s, &[&full[..a], &full[a..b], &full[b..]]);
        assert!(fwd.is_empty(), "3-way split leaked bytes: {fwd:?}");
        assert_eq!(evs, vec!["working".to_string()]);
    }

    /// Two complete sequences in a single chunk: both strip, both emit.
    #[test]
    fn two_sequences_in_one_chunk() {
        let mut s = scanner();
        let mut chunk = good_seq("working");
        chunk.extend_from_slice(&good_seq("awaiting_input"));
        let out = s.feed(&chunk);
        assert!(
            out.forward.is_empty(),
            "two-in-one leaked: {:?}",
            out.forward
        );
        assert_eq!(
            out.events,
            vec!["working".to_string(), "awaiting_input".to_string()]
        );
    }

    /// Normal output before, between, and after sequences is forwarded in order
    /// while the sequences themselves are stripped.
    #[test]
    fn interleaved_normal_output_preserved() {
        let mut s = scanner();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"before ");
        chunk.extend_from_slice(&good_seq("working"));
        chunk.extend_from_slice(b" middle ");
        chunk.extend_from_slice(&good_seq("awaiting_input"));
        chunk.extend_from_slice(b" after");
        let out = s.feed(&chunk);
        assert_eq!(out.forward, b"before  middle  after");
        assert_eq!(
            out.events,
            vec!["working".to_string(), "awaiting_input".to_string()]
        );
    }

    /// Foreign OSCs must pass through byte-for-byte — the scanner only ever
    /// engages our namespace. Covers OSC 0 (title), 8 (hyperlink), 9, 11 (bg),
    /// 99, and the near-namespace `777;notify`.
    #[test]
    fn foreign_osc_sequences_pass_through() {
        let cases: &[&[u8]] = &[
            b"\x1b]0;my window title\x07",
            b"\x1b]8;;https://example.com/foo\x07link\x1b]8;;\x07",
            b"\x1b]9;a notification body\x07",
            b"\x1b]11;?\x07",
            b"\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\",
            b"\x1b]99;i=1:d=0;hello\x07",
            b"\x1b]777;notify;title;body\x07",
        ];
        for case in cases {
            let mut s = scanner();
            let out = s.feed(case);
            assert_eq!(
                out.forward, *case,
                "foreign OSC was altered: {case:?} -> {:?}",
                out.forward
            );
            assert!(
                out.events.is_empty(),
                "foreign OSC emitted an event: {case:?}"
            );
            assert!(s.residual.is_empty(), "foreign OSC left residual: {case:?}");
        }
    }

    /// A bare `ESC` that is not our OSC (here, a CSI SGR reset) passes through
    /// and a following real sequence right after it still parses.
    #[test]
    fn foreign_escape_then_real_sequence() {
        let mut s = scanner();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"\x1b[0m"); // SGR reset — ESC not followed by `]`
        chunk.extend_from_slice(&good_seq("working"));
        let out = s.feed(&chunk);
        assert_eq!(out.forward, b"\x1b[0m");
        assert_eq!(out.events, vec!["working".to_string()]);
    }

    /// A stray `ESC ]` that diverges, immediately followed by a genuine
    /// sequence: the false start is flushed untouched and the real sequence
    /// still parses (the introducer-restart path).
    #[test]
    fn diverging_prefix_then_restart() {
        let mut s = scanner();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"\x1b]"); // opens like us, then diverges into ESC
        chunk.extend_from_slice(&good_seq("awaiting_approval"));
        let out = s.feed(&chunk);
        assert_eq!(
            out.forward, b"\x1b]",
            "the false start must survive verbatim"
        );
        assert_eq!(out.events, vec!["awaiting_approval".to_string()]);
    }

    /// A "sequence" that never terminates within the residual bound is flushed
    /// untouched (fail-open), losing no bytes, and scanning resumes so a later
    /// real sequence still parses.
    #[test]
    fn overflow_flushes_untouched() {
        let mut s = scanner();
        let mut runaway = Vec::new();
        runaway.extend_from_slice(b"\x1b]777;demeteo;");
        runaway.resize(runaway.len() + 200, b'x'); // 200 params, no terminator
        let out = s.feed(&runaway);
        assert_eq!(
            out.forward, runaway,
            "overflow must flush every byte untouched"
        );
        assert!(out.events.is_empty());
        assert!(
            s.residual.is_empty(),
            "overflow must reset to normal scanning"
        );

        // A real sequence after the overflow still parses.
        let out2 = s.feed(&good_seq("working"));
        assert!(out2.forward.is_empty());
        assert_eq!(out2.events, vec!["working".to_string()]);
    }

    /// Nonce accept: the matching nonce yields an event (baseline for the reject
    /// case below); parameter order is not significant.
    #[test]
    fn nonce_accepts_regardless_of_param_order() {
        let mut s = scanner();
        // state first, nonce last, v in the middle — order-insensitive parse.
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"\x1b]777;demeteo;state=working;v=1;nonce=");
        chunk.extend_from_slice(NONCE.as_bytes());
        chunk.push(BEL);
        let out = s.feed(&chunk);
        assert!(out.forward.is_empty());
        assert_eq!(out.events, vec!["working".to_string()]);
    }

    /// Nonce reject: a sequence carrying a *different* nonce is still stripped
    /// (no artifact) but produces no event — the anti-spoof gate.
    #[test]
    fn nonce_mismatch_strips_but_does_not_emit() {
        let mut s = scanner();
        let out = s.feed(&seq_with_nonce("deadbeef", "awaiting_approval", true));
        assert!(
            out.forward.is_empty(),
            "rejected sequence must still be stripped"
        );
        assert!(out.events.is_empty(), "a foreign nonce must not emit");
    }

    /// A missing nonce cannot match, so it strips without emitting.
    #[test]
    fn missing_nonce_strips_but_does_not_emit() {
        let mut s = scanner();
        let out = s.feed(b"\x1b]777;demeteo;v=1;state=working\x07");
        assert!(out.forward.is_empty());
        assert!(out.events.is_empty());
    }

    /// A sequence missing `state` is stripped (no artifact) but emits nothing.
    #[test]
    fn missing_state_strips_but_does_not_emit() {
        let mut s = scanner();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"\x1b]777;demeteo;v=1;nonce=");
        chunk.extend_from_slice(NONCE.as_bytes());
        chunk.push(BEL);
        let out = s.feed(&chunk);
        assert!(out.forward.is_empty(), "missing-state must still strip");
        assert!(out.events.is_empty());
    }

    /// An unrecognised version is stripped but not surfaced (forward-compat:
    /// ambiguity resolved as "strip, emit nothing").
    #[test]
    fn unknown_version_strips_but_does_not_emit() {
        let mut s = scanner();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"\x1b]777;demeteo;v=2;nonce=");
        chunk.extend_from_slice(NONCE.as_bytes());
        chunk.extend_from_slice(b";state=working\x07");
        let out = s.feed(&chunk);
        assert!(out.forward.is_empty(), "unknown-version must still strip");
        assert!(out.events.is_empty());
    }

    /// A sequence with no `v` field at all is accepted (default `v=1`).
    #[test]
    fn absent_version_defaults_to_accept() {
        let mut s = scanner();
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"\x1b]777;demeteo;nonce=");
        chunk.extend_from_slice(NONCE.as_bytes());
        chunk.extend_from_slice(b";state=awaiting_input\x07");
        let out = s.feed(&chunk);
        assert!(out.forward.is_empty());
        assert_eq!(out.events, vec!["awaiting_input".to_string()]);
    }

    /// The residual never exceeds the documented bound while collecting a
    /// runaway sequence — asserted byte-by-byte through the growth window.
    #[test]
    fn residual_stays_bounded() {
        let mut s = scanner();
        // Introducer engages the collecting phase; then feed one byte at a time
        // and assert the residual never crosses the cap before it flushes.
        let _ = s.feed(b"\x1b]777;demeteo;");
        for _ in 0..(MAX_RESIDUAL_BYTES * 2) {
            let _ = s.feed(b"x");
            assert!(
                s.residual.len() <= MAX_RESIDUAL_BYTES,
                "residual grew to {} — past the {MAX_RESIDUAL_BYTES}B cap",
                s.residual.len()
            );
        }
    }
}
