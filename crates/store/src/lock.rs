//! `store/LOCK`: an advisory `flock` held for the life of the process. A crashed server releases
//! it automatically; a stale file never blocks. The file's content is the holder's pid (for
//! humans and error messages).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use crate::error::{Result, StoreError};

/// Held lock on a `store/` directory. Dropping it releases the lock.
#[derive(Debug)]
pub struct StoreLock {
    _file: File,
    path: PathBuf,
}

impl StoreLock {
    /// Acquire `<store_dir>/LOCK` exclusively (non-blocking). `Err(Locked)` when another process
    /// holds it, with that process's pid when it wrote one.
    pub fn acquire(store_dir: &Path) -> Result<StoreLock> {
        std::fs::create_dir_all(store_dir)?;
        let path = store_dir.join(crate::LOCK_FILE);
        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
        // SAFETY: flock on a valid, open file descriptor owned by `file`.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                let mut s = String::new();
                let _ = file.read_to_string(&mut s);
                let pid = s.trim().parse::<u32>().ok();
                return Err(StoreError::Locked { path: store_dir.to_path_buf(), pid });
            }
            return Err(err.into());
        }
        file.set_len(0)?;
        file.rewind()?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_all().ok();
        Ok(StoreLock { _file: file, path })
    }

    /// Path of the lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_refused_with_pid_until_released() {
        let dir = tempfile::tempdir().unwrap();
        let first = StoreLock::acquire(dir.path()).unwrap();
        match StoreLock::acquire(dir.path()) {
            Err(StoreError::Locked { pid, .. }) => assert_eq!(pid, Some(std::process::id())),
            other => panic!("expected Locked, got {other:?}"),
        }
        drop(first);
        StoreLock::acquire(dir.path()).unwrap();
    }
}
