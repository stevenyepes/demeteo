//! The SFTP file operations of the SSH adapter: read/write/stat/list over the
//! pooled SFTP channel. Each one is the same shape — take the pooled session,
//! lock its `Sftp`, run one operation, and evict the session if *that* call
//! fails — so the shape lives in [`with_sftp`] / [`evict_on_err`] once. The
//! `ExecutionPort` impl in `client.rs` keeps only the `spawn_blocking`
//! wrappers. The `readdir`/`stat` post-processing is pure and lives here so it
//! is unit-testable without a live socket.

use super::session::SessionPool;
use crate::ports::execution::SftpEntry;
use ssh2::{FileStat, Sftp};
use std::fmt::Display;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Take the pooled session for `machine_id` and run `op` against its locked
/// `Sftp` handle. Every operation below shares this preamble.
fn with_sftp<T>(
    pool: &SessionPool,
    machine_id: &str,
    op: impl FnOnce(&Sftp) -> Result<T, String>,
) -> Result<T, String> {
    let sftp_sess = pool.get(machine_id)?;
    let sftp = sftp_sess
        .sftp
        .lock()
        .map_err(|_| "Failed to lock SFTP".to_string())?;
    op(&sftp)
}

/// Evict the pooled session before reporting: these failures mean the SFTP
/// channel itself is suspect, so the next caller should reconnect.
///
/// Applied only to the *first* SFTP call of each operation
/// (`open`/`create`/`stat`/`readdir`) — a later read/write/flush error is a
/// file-level problem and leaves the session pooled.
fn evict_on_err<T, E: Display>(
    pool: &SessionPool,
    machine_id: &str,
    r: Result<T, E>,
    ctx: &str,
) -> Result<T, String> {
    r.map_err(|e| {
        pool.evict(machine_id);
        format!("{}: {}", ctx, e)
    })
}

pub(super) fn read_file(
    pool: &SessionPool,
    machine_id: &str,
    path: &str,
) -> Result<String, String> {
    with_sftp(pool, machine_id, |sftp| {
        let path_buf = Path::new(path);
        let mut file = evict_on_err(pool, machine_id, sftp.open(path_buf), "Failed to open file")?;

        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .map_err(|e| format!("Failed to read file content: {}", e))?;
        Ok(contents)
    })
}

pub(super) fn write_file(
    pool: &SessionPool,
    machine_id: &str,
    path: &str,
    content: &str,
) -> Result<(), String> {
    write_file_bytes(pool, machine_id, path, content.as_bytes())
}

pub(super) fn write_file_bytes(
    pool: &SessionPool,
    machine_id: &str,
    path: &str,
    content: &[u8],
) -> Result<(), String> {
    with_sftp(pool, machine_id, |sftp| {
        let path_buf = Path::new(path);
        let mut file = evict_on_err(
            pool,
            machine_id,
            sftp.create(path_buf),
            "Failed to create file",
        )?;

        file.write_all(content)
            .map_err(|e| format!("Failed to write file: {}", e))?;
        file.flush()
            .map_err(|e| format!("Failed to flush file: {}", e))?;
        Ok(())
    })
}

pub(super) fn get_metadata(
    pool: &SessionPool,
    machine_id: &str,
    path: &str,
) -> Result<SftpEntry, String> {
    with_sftp(pool, machine_id, |sftp| {
        let path_buf = Path::new(path);
        let stat = evict_on_err(pool, machine_id, sftp.stat(path_buf), "Failed to stat file")?;

        let name = path_buf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let size = stat.size.unwrap_or(0);
        let modified = stat.mtime.unwrap_or(0);
        let is_dir = stat.is_dir();

        // `path` is the caller's original string rather than the stat'd
        // `PathBuf`, so a relative request comes back as it was asked for.
        Ok(SftpEntry {
            name,
            path: path.to_string(),
            is_dir,
            size,
            modified,
        })
    })
}

pub(super) fn list_dir(
    pool: &SessionPool,
    machine_id: &str,
    path: &str,
) -> Result<Vec<SftpEntry>, String> {
    with_sftp(pool, machine_id, |sftp| {
        let path_buf = Path::new(path);
        let entries = evict_on_err(
            pool,
            machine_id,
            sftp.readdir(path_buf),
            "Failed to read directory",
        )?;

        Ok(entries_from_readdir(entries))
    })
}

/// Map one readdir/stat pair to an `SftpEntry`. Pure.
fn entry_from_stat(path: &Path, stat: &FileStat) -> SftpEntry {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let path_str = path.to_str().unwrap_or("").to_string();
    let size = stat.size.unwrap_or(0);
    let modified = stat.mtime.unwrap_or(0);
    let is_dir = stat.is_dir();

    SftpEntry {
        name,
        path: path_str,
        is_dir,
        size,
        modified,
    }
}

/// Filter `.`/`..`, map, and order: directories first, then by name. Pure.
fn entries_from_readdir(raw: Vec<(PathBuf, FileStat)>) -> Vec<SftpEntry> {
    let mut list = Vec::new();
    for (p, stat) in raw {
        let entry = entry_from_stat(&p, &stat);

        // Kept verbatim from the pre-split code, but note it is a belt-and-
        // braces guard rather than the thing keeping dot entries out of the
        // browser: `Sftp::readdir` already drops `.`/`..` upstream, and
        // `Path::file_name` never yields either name anyway (it resolves
        // `dir/.` to "dir" and `dir/..` to `None`). See the tests below.
        if entry.name == "." || entry.name == ".." {
            continue;
        }

        list.push(entry);
    }

    list.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            b.is_dir.cmp(&a.is_dir)
        } else {
            a.name.cmp(&b.name)
        }
    });

    list
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `S_IFDIR | 0755` — what a remote directory's `perm` looks like, and the
    /// only field `FileStat::is_dir` consults.
    const DIR_PERM: u32 = 0o040_755;
    /// `S_IFREG | 0644`.
    const FILE_PERM: u32 = 0o100_644;

    fn stat(perm: u32, size: Option<u64>, mtime: Option<u64>) -> FileStat {
        FileStat {
            size,
            uid: None,
            gid: None,
            perm: Some(perm),
            atime: None,
            mtime,
        }
    }

    fn dir(name: &str) -> (PathBuf, FileStat) {
        (
            PathBuf::from(format!("/remote/{}", name)),
            stat(DIR_PERM, Some(4096), Some(1_700_000_000)),
        )
    }

    fn file(name: &str) -> (PathBuf, FileStat) {
        (
            PathBuf::from(format!("/remote/{}", name)),
            stat(FILE_PERM, Some(12), Some(1_700_000_001)),
        )
    }

    /// `readdir` reports `.` and `..` on most servers and surfacing them would
    /// put bogus rows in the file browser — but what keeps them out is
    /// `Sftp::readdir`, which drops both before we ever see them. The
    /// name-based guard here can never fire, because `Path::file_name`
    /// resolves `dir/.` to "dir" and `dir/..` to `None`. Pin that so nobody
    /// deletes the upstream reliance believing this guard covers it.
    #[test]
    fn the_dot_guard_never_fires_because_file_name_never_yields_a_dot() {
        let entries = entries_from_readdir(vec![
            (PathBuf::from("/remote/."), stat(DIR_PERM, None, None)),
            (PathBuf::from("/remote/.."), stat(DIR_PERM, None, None)),
            file("a.txt"),
        ]);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["", "remote", "a.txt"],
            "dot paths pass the guard with a resolved or empty name",
        );
    }

    /// Directories sort ahead of files regardless of name, and each group is
    /// then ascending by name — the order the file browser renders verbatim.
    #[test]
    fn directories_sort_before_files_then_by_name() {
        let raw = vec![
            file("alpha.txt"),
            dir("zeta"),
            file("beta.txt"),
            dir("apple"),
        ];
        let entries = entries_from_readdir(raw);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["apple", "zeta", "alpha.txt", "beta.txt"],
        );
    }

    /// `FileStat`'s fields are all optional — a server that omits size or mtime
    /// must yield 0, not a panic and not a stale value from another entry.
    #[test]
    fn missing_size_and_mtime_default_to_zero() {
        let entries = entries_from_readdir(vec![(
            PathBuf::from("/remote/sparse.bin"),
            stat(FILE_PERM, None, None),
        )]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 0);
        assert_eq!(entries[0].modified, 0);
        assert_eq!(entries[0].path, "/remote/sparse.bin");
        assert!(!entries[0].is_dir);
    }

    /// The full path comes from the readdir entry, the name from its last
    /// component, and `is_dir` from the mode bits.
    #[test]
    fn entry_carries_path_name_and_dir_flag() {
        let entry = entry_from_stat(
            Path::new("/remote/build"),
            &stat(DIR_PERM, Some(4096), Some(42)),
        );
        assert_eq!(entry.name, "build");
        assert_eq!(entry.path, "/remote/build");
        assert!(entry.is_dir);
        assert_eq!(entry.size, 4096);
        assert_eq!(entry.modified, 42);
    }
}
