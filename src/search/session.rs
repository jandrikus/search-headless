//! One private Brave profile and challenge cooldown shared by CLI processes.
use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::types::{Deadline, ToolError};

pub const CHALLENGE_COOLDOWN_MS: u64 = 15 * 60_000;

pub struct SearchSession {
    // The OS releases this lock even if the CLI is killed. Hold it through the
    // browser exit and cooldown write so queued searches see the new state.
    _lock: File,
    dir: PathBuf,
    pub storage_dir: PathBuf,
}

impl SearchSession {
    pub async fn acquire(dir: &Path, deadline: &Deadline) -> Result<Self, ToolError> {
        private_dir(dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.join("session.lock"))
            .map_err(state_error)?;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    if deadline.remaining_ms() <= 50 {
                        return Err(ToolError::new(
                            "FETCH_TIMEOUT",
                            "Timed out waiting for the shared Brave session.",
                            true,
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(TryLockError::Error(error)) => return Err(state_error(error)),
            }
        }
        let session = Self { _lock: lock, dir: dir.to_path_buf(), storage_dir: dir.join("profile") };
        if let Some(until) = session.cooldown_until()? {
            let remaining = until.saturating_sub(now_ms());
            if remaining > 0 {
                let seconds = remaining.div_ceil(1000);
                return Err(ToolError::new(
                    "BRAVE_COOLDOWN",
                    format!("Brave search is paused after a bot challenge. Retry in {seconds}s; no request was sent."),
                    true,
                )
                .with_details(serde_json::json!({ "retryAfterSeconds": seconds, "nextAllowedAtMs": until })));
            }
        }
        private_dir(&session.storage_dir)?;
        Ok(session)
    }

    fn cooldown_until(&self) -> Result<Option<u64>, ToolError> {
        match fs::read_to_string(self.dir.join("cooldown-until")) {
            Ok(raw) => raw.trim().parse().map(Some).map_err(|_| {
                ToolError::new("BRAVE_STATE_ERROR", "The Brave cooldown file contains an invalid timestamp.", false)
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(state_error(error)),
        }
    }

    pub fn record_challenge(&self) -> Result<u64, ToolError> {
        let until = now_ms().saturating_add(CHALLENGE_COOLDOWN_MS);
        let temporary = self.dir.join("cooldown-until.tmp");
        fs::write(&temporary, until.to_string()).map_err(state_error)?;
        fs::rename(temporary, self.dir.join("cooldown-until")).map_err(state_error)?;
        Ok(until)
    }
}

fn private_dir(path: &Path) -> Result<(), ToolError> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path).map_err(state_error)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(state_error)
}

fn state_error(error: std::io::Error) -> ToolError {
    ToolError::new("BRAVE_STATE_ERROR", format!("Cannot access the shared Brave session: {error}"), false)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
