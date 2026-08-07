use crate::domain::models::Platform;

#[test]
fn from_target_os_round_trips_as_str() {
    for platform in Platform::ALL {
        assert_eq!(Platform::from_target_os(platform.as_str()), Some(platform));
    }
}

#[test]
fn from_target_os_rejects_a_host_demeteo_does_not_ship_for() {
    for os in ["freebsd", "illumos", "android", "Linux", "", "win32"] {
        assert_eq!(Platform::from_target_os(os), None, "accepted {os:?}");
    }
}

#[test]
fn from_uname_reads_the_two_posix_kernels() {
    assert_eq!(Platform::from_uname("Linux"), Some(Platform::Linux));
    assert_eq!(Platform::from_uname("Darwin"), Some(Platform::MacOS));
    assert_eq!(Platform::from_uname("Darwin\n"), Some(Platform::MacOS));
}

/// An MSYS `uname` is the shape that would arrive if a Windows box were
/// registered as an SSH machine, which remote execution does not support. It
/// has to read as "not a platform I can name" rather than as Windows.
#[test]
fn from_uname_rejects_everything_that_is_not_linux_or_darwin() {
    for sysname in [
        "MINGW64_NT-10.0-22631",
        "CYGWIN_NT-10.0",
        "FreeBSD",
        "linux",
        "",
    ] {
        assert_eq!(
            Platform::from_uname(sysname),
            None,
            "accepted {sysname:?} as a platform",
        );
    }
}

#[test]
fn only_windows_is_not_posix() {
    assert!(Platform::Linux.is_posix());
    assert!(Platform::MacOS.is_posix());
    assert!(!Platform::Windows.is_posix());
}

#[test]
fn serde_uses_the_canonical_lowercase_spelling() {
    for platform in Platform::ALL {
        assert_eq!(
            serde_json::to_string(&platform).unwrap(),
            format!("\"{}\"", platform.as_str())
        );
        assert_eq!(
            serde_json::from_str::<Platform>(&format!("\"{}\"", platform.as_str())).unwrap(),
            platform
        );
    }
}
