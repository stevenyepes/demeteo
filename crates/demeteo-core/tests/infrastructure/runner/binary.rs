// Tests extracted from `crates/demeteo-core/src/infrastructure/runner/binary.rs` (mirrored-tests convention). `super` = that module.

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

fn built(version: Option<&str>) -> RunnerBinary {
    RunnerBinary {
        path: PathBuf::from("demeteo-runner"),
        version: version.map(str::to_string),
    }
}

#[test]
fn stale_warning_is_none_for_the_same_version() {
    assert_eq!(
        stale_version_warning(&built(Some("1.2.0-31")), "1.2.0-31"),
        None
    );
}

#[test]
fn stale_warning_treats_the_raw_version_line_as_its_version() {
    let binary = built(Some("demeteo-runner 1.2.0-31"));
    assert_eq!(stale_version_warning(&binary, "1.2.0-31"), None);
}

#[test]
fn stale_warning_is_none_when_the_version_could_not_be_read() {
    assert_eq!(stale_version_warning(&built(None), "1.2.0"), None);
}

#[test]
fn stale_warning_for_an_older_build_says_upgrade_the_runner() {
    let warning = stale_version_warning(&built(Some("1.1.0")), "1.2.0").unwrap();
    assert!(warning.contains("older"), "{warning}");
    assert!(warning.contains("upgrade the runner"), "{warning}");
    assert!(!warning.contains("newer"), "{warning}");
}

#[test]
fn stale_warning_for_a_newer_build_says_upgrade_demeteo() {
    // A nightly of the same X.Y.Z is built after the stable, so it is ahead.
    let warning = stale_version_warning(&built(Some("1.2.0-3")), "1.2.0").unwrap();
    assert!(warning.contains("newer"), "{warning}");
    assert!(warning.contains("upgrade Demeteo"), "{warning}");
    assert!(!warning.contains("older"), "{warning}");
}

#[test]
fn stale_warning_still_fires_for_an_unreadable_version() {
    let warning = stale_version_warning(&built(Some("garbage")), "1.2.0").unwrap();
    assert!(warning.contains("garbage"), "{warning}");
}

#[test]
fn version_output_is_read_through_the_domain_parser() {
    assert_eq!(
        version_from_output("demeteo-runner 1.2.0-31\n").as_deref(),
        Some("1.2.0-31")
    );
    assert_eq!(
        version_from_output("  weird output ").as_deref(),
        Some("weird output")
    );
    assert_eq!(version_from_output(" \n"), None);
}
