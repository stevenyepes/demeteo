//! Per-feature attachment domain model.
//!
//! Attachments are user-supplied files (typically screenshots or short
//! documents) that the user dropped or picked in the Start-Feature modal
//! before launching a feature. They're persisted on disk under
//! `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>` and
//! copied into the per-step worktree's `artifacts/_context/` directory
//! right before the agent's turn so they sit inside opencode's
//! `external_directory: deny` fence.
//!
//! The content-addressable on-disk layout (`<sha256>.<ext>`) makes a
//! re-upload of the same file a no-op (the bytes are already on disk
//! under that hash), and lets the orchestrator emit stable path
//! manifests in the rendered prompt even if the user re-attaches the
//! same image repeatedly.
//!
//! No external crate is used for the SHA-256 / hex encoding — the
//! algorithm is fixed by NIST FIPS 180-4 and the project keeps the
//! dependency graph tight. `compute_sha256_hex` is exposed for tests
//! and for callers that need the hash of an already-read buffer.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachedFile {
    pub id: String,
    /// The display name shown in chips / headers. Sanitized at
    /// ingest so it can never contain a path separator or null byte.
    pub name: String,
    pub mime: String,
    /// Hex-encoded SHA-256 (lowercase, 64 chars). Used as the
    /// content-addressable on-disk filename together with `ext`.
    pub sha256: String,
    pub size: u64,
    /// The user's original filename (before sanitization). Kept
    /// around so the UI can show "Screenshot 2025-01-02.png" instead
    /// of the sanitized `Screenshot_2025_01_02_png`.
    pub source_filename: String,
}

impl AttachedFile {
    pub fn on_disk_path(&self, attachments_dir: &std::path::Path, ext: &str) -> std::path::PathBuf {
        attachments_dir.join(format!("{}.{}", self.sha256, ext))
    }
}

/// Sanitize a user-supplied attachment filename so it can be used
/// safely as part of an on-disk path (no directory traversal, no
/// shell-interpretation fun). Mirrors the rule used by
/// `adapters::artifact_store::fs::sanitize`: every character that
/// isn't ASCII alphanumeric, `-`, or `_` becomes `_`.
///
/// The function preserves the case of letters so users with
/// `MyScreenshot.png` get `MyScreenshot.png`, not
/// `myscreenshot.png`. The leading character is forced to
/// `[A-Za-z0-9_]` so the result never starts with `-` (which the
/// shell would interpret as a flag).
pub fn sanitize_attachment_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        let mapped = if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            c
        } else {
            '_'
        };
        if i == 0 && (mapped == '-' || mapped == '.') {
            out.push('_');
        } else {
            out.push(mapped);
        }
    }
    if out.is_empty() {
        "attachment".to_string()
    } else {
        out
    }
}

/// Lowercase extension → recommended on-disk extension. `None` means
/// the extension is unknown; the caller should fall back to `bin`.
pub fn ext_for_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "text/plain" => Some("txt"),
        "text/markdown" => Some("md"),
        "application/json" => Some("json"),
        "application/pdf" => Some("pdf"),
        _ => None,
    }
}

/// Lowercase extension → IANA mime guess.
pub fn mime_for_ext(ext: &str) -> Option<&'static str> {
    let e = ext.to_ascii_lowercase();
    match e.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "txt" => Some("text/plain"),
        "md" | "markdown" => Some("text/markdown"),
        "json" => Some("application/json"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

/// Compute SHA-256 over `bytes` and return it as 64 lowercase hex
/// characters. Pure-Rust NIST FIPS 180-4 implementation — no
/// dependency required.
pub fn compute_sha256_hex(bytes: &[u8]) -> String {
    let mut state = Sha256State::new();
    state.update(bytes);
    state.finalize_hex()
}

struct Sha256State {
    h: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha256State {
    fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.buf_len = 0;
                compress(&mut self.h, &block);
            }
        }
    }

    fn finalize_hex(mut self) -> String {
        // Padding: append 0x80, zeros, then 8-byte big-endian length.
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            for b in &mut self.buf[self.buf_len..] {
                *b = 0;
            }
            let block = self.buf;
            self.buf_len = 0;
            compress(&mut self.h, &block);
        }
        for b in &mut self.buf[self.buf_len..56] {
            *b = 0;
        }
        let bit_len = self.total_len.wrapping_mul(8);
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.buf;
        compress(&mut self.h, &block);

        let mut out = String::with_capacity(64);
        for word in &self.h {
            out.push_str(&format!("{:08x}", word));
        }
        out
    }
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn compress(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let mut a = h[0];
    let mut b = h[1];
    let mut c = h[2];
    let mut d = h[3];
    let mut e = h[4];
    let mut f = h[5];
    let mut g = h[6];
    let mut hh = h[7];

    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let temp1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_separators() {
        // Each non-alphanumeric, non-`-`/`_` char becomes `_`.
        // `..` → `__`, `/` → `_`, `\..evil.txt` → `_` + alphanumeric + `_txt`
        assert_eq!(
            sanitize_attachment_filename("../etc/passwd"),
            "___etc_passwd"
        );
        assert_eq!(sanitize_attachment_filename("..\\evil.txt"), "___evil_txt");
    }

    #[test]
    fn sanitize_strips_null_bytes() {
        assert_eq!(sanitize_attachment_filename("name\0.png"), "name__png");
    }

    #[test]
    fn sanitize_keeps_unicode() {
        // Unicode chars are mapped to '_' (non-ascii alphanumeric).
        assert_eq!(sanitize_attachment_filename("café.png"), "caf__png");
    }

    #[test]
    fn sanitize_handles_empty_input() {
        assert_eq!(sanitize_attachment_filename(""), "attachment");
        assert_eq!(sanitize_attachment_filename("..."), "___");
        // Leading dash replaced
        assert!(sanitize_attachment_filename("-evil").starts_with('_'));
    }

    #[test]
    fn mime_for_ext_known_types() {
        assert_eq!(mime_for_ext("png"), Some("image/png"));
        assert_eq!(mime_for_ext("JPG"), Some("image/jpeg"));
        assert_eq!(mime_for_ext("xyz"), None);
    }

    #[test]
    fn compute_sha256_known_vector() {
        // "abc" → ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let hex = compute_sha256_hex(b"abc");
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn compute_sha256_empty_input() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let hex = compute_sha256_hex(b"");
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn compute_sha256_long_input() {
        // Million-a test: compute SHA-256 over a million 'a' bytes.
        // Verified against Python's hashlib (NIST reference).
        let buf = vec![b'a'; 1_000_000];
        let hex = compute_sha256_hex(&buf);
        assert_eq!(
            hex,
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn attached_file_on_disk_path() {
        let f = AttachedFile {
            id: "at-1".into(),
            name: "shot".into(),
            mime: "image/png".into(),
            sha256: "abc123".into(),
            size: 42,
            source_filename: "shot.png".into(),
        };
        assert_eq!(
            f.on_disk_path(std::path::Path::new("/tmp/att"), "png"),
            std::path::PathBuf::from("/tmp/att/abc123.png")
        );
    }

    #[test]
    fn attached_file_serde_roundtrip() {
        let f = AttachedFile {
            id: "at-1".into(),
            name: "shot".into(),
            mime: "image/png".into(),
            sha256: "abc123".into(),
            size: 42,
            source_filename: "shot.png".into(),
        };
        let json = serde_json::to_string(&f).unwrap();
        let back: AttachedFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }
}
