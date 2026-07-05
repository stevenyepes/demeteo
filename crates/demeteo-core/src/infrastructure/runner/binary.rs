//! Locate, classify, and probe a `demeteo-runner` binary sitting on
//! this laptop. Three concerns live here:
//!
//! 1. *Where* to find one — three-tier lookup: dev cache written by
//!    `scripts/build-runner.sh`, `$DEMETEO_RUNNER_BIN` override, the
//!    directory the running app binary lives in. Plus the version-
//!    keyed cache used by `download::release`.
//! 2. *Whether* it's usable — magic-byte arch detection catches the
//!    recurring mistake of a Mac dev pushing an arm64 Mach-O binary
//!    to a Linux x86_64 host (`Exec format error`).
//! 3. *Which* version it reports — best-effort `<path> --version` with
//!    a short timeout, used to warn about stale dev builds *before*
//!    pushing them.

use crate::error::AppError;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Mirrors the asset name CI uploads in `.github/workflows/build.yml`.
pub const RUNNER_ASSET_NAME: &str = "demeteo-runner-x86_64-unknown-linux-musl";

/// A local `demeteo-runner` binary we could push, with whatever version
/// string we could read off it.
#[derive(Debug, Clone)]
pub struct RunnerBinary {
    pub path: PathBuf,
    pub version: Option<String>,
}

/// Coarse architecture classification derived from the file's magic
/// bytes. Anything we don't recognise as Linux x86_64 ELF is treated
/// as not-pushable; `MacOs` covers the recurring Mac dev laptop case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerArch {
    LinuxX86_64,
    LinuxOther,
    MacOs,
    Windows,
    Unknown,
}

/// Three-tier lookup, in priority order:
///
/// 1. **Dev cache** (`<tmpdir>/demeteo-runner-cache/dev/demeteo-runner-x86_64-unknown-linux-musl`)
///    — written by `npm run build:runner` / `scripts/build-runner.sh`,
///    guaranteed Linux x86_64 because the script uses the musl target.
/// 2. **`$DEMETEO_RUNNER_BIN`** — explicit override; trusted if it
///    exists, classified by magic bytes on demand.
/// 3. **`<app-dir>/demeteo-runner`** — sibling of the running Tauri
///    binary; the "I just ran `cargo build`" path. Classified the same
///    way; the magic-byte guard catches Mach-O here.
///
/// Returns `None` when none of the three resolve to a real file —
/// callers fall through to the release-download path.
pub async fn locate_local() -> Option<RunnerBinary> {
    let dev = dev_cache_path();
    if dev.is_file() {
        return Some(read_with_version(dev).await);
    }
    if let Ok(p) = std::env::var("DEMETEO_RUNNER_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(read_with_version(path).await);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join("demeteo-runner");
    if candidate.is_file() {
        Some(read_with_version(candidate).await)
    } else {
        None
    }
}

/// Path `scripts/build-runner.sh` writes dev builds to. Same parent as
/// `release_cache_path` so a single `rm -rf <tmpdir>/demeteo-runner-cache`
/// clears both.
pub fn dev_cache_path() -> PathBuf {
    std::env::temp_dir()
        .join("demeteo-runner-cache")
        .join("dev")
        .join(RUNNER_ASSET_NAME)
}

/// Version-keyed cache for `download::release` — one file per version
/// so flipping between stable/nightly never re-downloads.
pub fn release_cache_path(version: &str) -> PathBuf {
    std::env::temp_dir()
        .join("demeteo-runner-cache")
        .join(version)
        .join(RUNNER_ASSET_NAME)
}

impl RunnerBinary {
    /// Read the first few bytes and classify. Returns `RunnerArch::Unknown`
    /// when the file can't be read or the magic doesn't match a known
    /// format — callers should treat that as "don't push, ask the user
    /// to rebuild".
    pub fn arch(&self) -> Result<RunnerArch, AppError> {
        arch_from_path(&self.path)
    }

    /// `true` only when the binary is a Linux x86_64 ELF, the only arch
    /// Demeteo can ship to a remote Linux host.
    pub fn is_linux_x86_64(&self) -> Result<bool, AppError> {
        Ok(self.arch()? == RunnerArch::LinuxX86_64)
    }
}

/// Read magic bytes; classify.
fn arch_from_path(path: &Path) -> Result<RunnerArch, AppError> {
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::from(format!("failed to read {}: {}", path.display(), e)))?;
    Ok(classify_magic(&bytes))
}

fn classify_magic(bytes: &[u8]) -> RunnerArch {
    if bytes.len() < 4 {
        return RunnerArch::Unknown;
    }
    // ELF: 0x7f 'E' 'L' 'F', followed by class (1=32, 2=64) and
    // endianness (1=LE, 2=BE) and OS/ABI (0=System V, 0x3=Linux).
    if &bytes[0..4] == b"\x7fELF" {
        if bytes.len() < 20 {
            return RunnerArch::LinuxOther;
        }
        let is_64 = bytes[4] == 2;
        let is_le = bytes[5] == 1;
        let osabi = bytes[7];
        if is_64 && is_le && (osabi == 0 || osabi == 0x03) {
            return RunnerArch::LinuxX86_64;
        }
        return RunnerArch::LinuxOther;
    }
    // Mach-O magic: 0xfeedface (32-bit BE), 0xfeedfacf (64-bit BE),
    // 0xcefaedfe (32-bit LE), 0xcffaedfe (64-bit LE — arm64/x86_64 on
    // macOS). All four indicate "macOS, do not push to a Linux box".
    let m = &bytes[0..4];
    if m == b"\xfe\xed\xfa\xce"
        || m == b"\xfe\xed\xfa\xcf"
        || m == b"\xce\xfa\xed\xfe"
        || m == b"\xcf\xfa\xed\xfe"
    {
        return RunnerArch::MacOs;
    }
    // PE / Windows .exe: 'M' 'Z' (DOS stub header — followed by more
    // bytes, but the magic is just the first two).
    if bytes.len() >= 2 && bytes[0] == b'M' && bytes[1] == b'Z' {
        return RunnerArch::Windows;
    }
    RunnerArch::Unknown
}

/// If `binary.version` is `Some(v)` and `v != expected`, returns a
/// human-readable warning explaining the mismatch. `None` when versions
/// match, when the version is unknown (binary wasn't executable on this
/// laptop, e.g. the musl build on a Mac dev), or when the binary is
/// absent.
pub fn stale_version_warning(binary: &RunnerBinary, expected: &str) -> Option<String> {
    match &binary.version {
        Some(v) if v.as_str() != expected => Some(format!(
            "this local build reports version {v}, not the app's {expected} — \
             it'll be pushed as-is"
        )),
        _ => None,
    }
}

/// Best-effort `<path> --version` with a short timeout. Returns `None`
/// on any failure (wrong arch on this laptop, missing exec bit,
/// timeout, non-zero exit) — callers never push based on this probe.
pub async fn probe_version(path: &Path) -> Option<String> {
    let output = match tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new(path).arg("--version").output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        _ => return None,
    };
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .rsplit(' ')
        .next()
        .map(|s| s.to_string())
}

async fn read_with_version(path: PathBuf) -> RunnerBinary {
    let version = probe_version(&path).await;
    RunnerBinary { path, version }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_path(name: &str, bytes: &[u8]) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "demeteo-runner-arch-test-{name}-{pid}",
            name = name,
            pid = std::process::id()
        ));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    #[test]
    fn elf_x86_64_linux() {
        let mut bytes = vec![0x7f, b'E', b'L', b'F'];
        bytes.extend_from_slice(&[2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let path = tmp_path("elf", &bytes);
        assert_eq!(arch_from_path(&path).unwrap(), RunnerArch::LinuxX86_64);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn elf_linux_32bit_is_other() {
        // 32-bit little-endian ELF: class=1, data=1, OSABI=0 → not
        // LinuxX86_64. We don't sniff e_machine, so any 32-bit ELF is
        // classified as LinuxOther rather than something more specific.
        let bytes = b"\x7fELF\x01\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let path = tmp_path("elf-32", bytes);
        assert_eq!(arch_from_path(&path).unwrap(), RunnerArch::LinuxOther);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn elf_be_is_other() {
        // 64-bit big-endian ELF: not x86_64 (LE).
        let bytes = b"\x7fELF\x02\x02\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let path = tmp_path("elf-be", bytes);
        assert_eq!(arch_from_path(&path).unwrap(), RunnerArch::LinuxOther);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn macho_arm64_le() {
        let bytes = b"\xcf\xfa\xed\xfe rest of header ignored";
        let path = tmp_path("macho", bytes);
        assert_eq!(arch_from_path(&path).unwrap(), RunnerArch::MacOs);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pe_windows() {
        // DOS stub header — only the first 2 bytes ("MZ") are magic.
        let bytes = b"MZ\x90\x00\x03\x00\x00\x00\x04\x00\x00\x00\xff\xff";
        let path = tmp_path("pe", bytes);
        assert_eq!(arch_from_path(&path).unwrap(), RunnerArch::Windows);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unknown_short() {
        let bytes = b"\x7fEL";
        let path = tmp_path("short", bytes);
        assert_eq!(arch_from_path(&path).unwrap(), RunnerArch::Unknown);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_errors() {
        let path = PathBuf::from("/nonexistent/demeteo-runner");
        assert!(arch_from_path(&path).is_err());
    }
}
