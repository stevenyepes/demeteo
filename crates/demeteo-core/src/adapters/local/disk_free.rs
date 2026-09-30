//! The syscall behind the local [`free_space_bytes`], and nothing else: every
//! decision about the answer is `domain::seed_space`'s, where a Linux test
//! reaches it.
//!
//! [`free_space_bytes`]: crate::ports::execution::ExecutionPort::free_space_bytes

#[cfg(unix)]
pub(super) fn available_bytes(path: &str) -> Result<u64, String> {
    let c_path = std::ffi::CString::new(path)
        .map_err(|_| format!("Failed to stat filesystem of '{path}': path holds a NUL"))?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Err(format!(
            "Failed to stat filesystem of '{path}': {}",
            std::io::Error::last_os_error()
        ));
    }
    #[allow(clippy::useless_conversion)]
    let (blocks, fragment) = (u64::from(stat.f_bavail), u64::from(stat.f_frsize));
    blocks
        .checked_mul(fragment)
        .ok_or_else(|| format!("Filesystem of '{path}' reported an out-of-range size"))
}

#[cfg(windows)]
pub(super) fn available_bytes(path: &str) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(format!(
            "Failed to stat filesystem of '{path}': {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(available)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn available_bytes(path: &str) -> Result<u64, String> {
    Err(format!(
        "Failed to stat filesystem of '{path}': not supported on this platform"
    ))
}
