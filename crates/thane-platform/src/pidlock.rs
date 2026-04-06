use std::fs::File;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

/// An advisory file lock that prevents multiple thane GUI instances from running.
///
/// Holds an exclusive `flock` on a pidfile in the runtime directory. The lock is
/// automatically released when the process exits (or the `PidLock` is dropped).
pub struct PidLock {
    _file: File,
    path: PathBuf,
}

impl PidLock {
    /// Try to acquire the single-instance lock.
    ///
    /// Returns `Ok(PidLock)` if this is the only running instance, or `Err` with
    /// the PID of the existing instance if another one holds the lock.
    pub fn acquire(runtime_dir: &std::path::Path) -> Result<Self, AcquireError> {
        std::fs::create_dir_all(runtime_dir)
            .map_err(|e| AcquireError::Io(e))?;

        let path = runtime_dir.join("thane.pid");

        let file = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(AcquireError::Io)?;

        // Try a non-blocking exclusive lock.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                // Another instance holds the lock — read its PID.
                let contents = std::fs::read_to_string(&path).unwrap_or_default();
                let pid = contents.trim().parse::<u32>().ok();
                return Err(AcquireError::AlreadyRunning(pid));
            }
            return Err(AcquireError::Io(err));
        }

        // We hold the lock — write our PID.
        let mut file = file;
        file.set_len(0).map_err(AcquireError::Io)?;
        write!(file, "{}", std::process::id()).map_err(AcquireError::Io)?;

        Ok(Self { _file: file, path })
    }
}

impl Drop for PidLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
pub enum AcquireError {
    /// Another thane instance is already running (with optional PID).
    AlreadyRunning(Option<u32>),
    /// Filesystem error.
    Io(std::io::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning(Some(pid)) => {
                write!(f, "Another thane instance is already running (PID {pid})")
            }
            Self::AlreadyRunning(None) => {
                write!(f, "Another thane instance is already running")
            }
            Self::Io(e) => write!(f, "Lock file error: {e}"),
        }
    }
}

impl std::error::Error for AcquireError {}
