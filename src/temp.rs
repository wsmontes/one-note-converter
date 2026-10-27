//! Per-operation scratch directory: no shared process-id path or destructive pre-cleanup.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct ScratchDir(PathBuf);

impl ScratchDir {
    pub fn new(prefix: &str) -> Result<Self, String> {
        let base = std::env::temp_dir();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock: {error}"))?
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{prefix}-{}-{epoch}-{id}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("cannot create {}: {error}", path.display())),
            }
        }
        Err("could not allocate a unique scratch directory".into())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
