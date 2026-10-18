// Single-instance guard for the TUI.
//
// Why: the config is loaded into memory at startup and rewritten on every
// favorite/history change. Two running instances therefore clobber each other,
// and an edit made outside the app is silently reverted by the running one
// (observed 2026-10-17: a playlist URL changed by hand was overwritten).
// The TUI now holds an exclusive flock for its whole lifetime; a second launch
// fails with a clear message instead of racing on config.json.
//
// Used by `main` (TUI startup) and by the `update` CLI, which only warns when
// the TUI holds the lock because it never writes the config.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;

pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Try to take the instance lock without blocking.
    pub fn acquire() -> Result<Self> {
        let dir = crate::consts::get_cache_dir();
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create cache dir {}", dir.display()))?;
        let path = dir.join("instance.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open lock file {}", path.display()))?;

        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
                anyhow::bail!(
                    "another instance is already running (lock: {}) — quit it first (q) or use `ip_tv-neon update` from a terminal",
                    path.display()
                );
            }
            return Err(err).with_context(|| format!("lock {}", path.display()));
        }

        let mut writer = &file;
        let _ = writer.set_len(0);
        let _ = writeln!(writer, "{}", std::process::id());

        Ok(Self { _file: file })
    }
}

/// True when another process currently holds the instance lock.
pub fn other_instance_running() -> bool {
    InstanceLock::acquire().is_err()
}
