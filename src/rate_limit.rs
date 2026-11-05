//! Cross-process spacing for Brave searches.
//!
//! The timestamp alone is not enough: two search processes started together both
//! read the same old timestamp, both compute the same wait, and both fire at the
//! same instant - exactly the burst the spacing exists to prevent. So the whole
//! read-wait-stamp sequence runs under an exclusive lock, which makes the second
//! process queue behind the first rather than race it.

use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config;
use crate::types::Deadline;

/// Minimum seconds between searches to appear human-like.
pub const MIN_SEARCH_INTERVAL_SEC: u64 = 10;

/// A lock older than this belonged to a process that died holding it.
const STALE_LOCK_MS: u64 = MIN_SEARCH_INTERVAL_SEC * 3 * 1000;

/// Timestamp of the last search, kept with the user's private Brave state.
pub fn last_search_file() -> PathBuf {
    match std::env::var("SEARCH_HEADLESS_RATE_FILE") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => config::search_state_dir().join("last-search"),
    }
}

pub struct RateLimitOptions {
    pub deadline: Deadline,
    pub interval_sec: Option<u64>,
    pub file: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RateLimitOutcome {
    /// A slot was taken; the search may run.
    Ok { waited_ms: u64 },
    /// The wait would outlast the caller's deadline; nothing was reserved.
    Declined { required_ms: u64 },
}

impl RateLimitOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, RateLimitOutcome::Ok { .. })
    }
}

fn disabled() -> bool {
    std::env::var("SEARCH_HEADLESS_NO_RATE_LIMIT").as_deref() == Ok("1")
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Take the lock, or return false if it is held by a live process.
fn try_lock(path: &Path) -> bool {
    match fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => true,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => break_stale_lock(path),
        Err(_) => false,
    }
}

fn break_stale_lock(path: &Path) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|m| m.modified()) else { return false };
    let Ok(age) = modified.elapsed() else { return false };
    if age.as_millis() as u64 <= STALE_LOCK_MS {
        return false;
    }
    if fs::remove_file(path).is_err() {
        return false;
    }
    // Someone else may have won the race to break it; create_new decides.
    fs::OpenOptions::new().write(true).create_new(true).open(path).is_ok()
}

/// Wait until the next search slot is free, then claim it.
///
/// Returns `Declined` rather than overrunning the caller's deadline, so a short
/// `--timeout` reports a timeout instead of silently blocking for ten seconds.
pub async fn reserve_search_slot(options: RateLimitOptions) -> RateLimitOutcome {
    if disabled() {
        return RateLimitOutcome::Ok { waited_ms: 0 };
    }

    let file = options.file.unwrap_or_else(last_search_file);
    let lock = PathBuf::from(format!("{}.lock", file.display()));
    let interval_ms = options.interval_sec.unwrap_or(MIN_SEARCH_INTERVAL_SEC) * 1000;
    let started_at = now_ms();

    if let Some(parent) = file.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Queue for the lock. Another process holding it is itself waiting out the
    // interval, so its wait counts against ours and is checked against the deadline.
    while !try_lock(&lock) {
        if options.deadline.remaining_ms() <= 250 {
            return RateLimitOutcome::Declined { required_ms: interval_ms };
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let outcome = claim(&file, interval_ms, &options.deadline, started_at).await;
    let _ = fs::remove_file(&lock);
    outcome
}

async fn claim(file: &Path, interval_ms: u64, deadline: &Deadline, started_at: u64) -> RateLimitOutcome {
    let last = fs::read_to_string(file).ok().and_then(|raw| raw.trim().parse::<u64>().ok());
    let wait_ms = last.map(|last| interval_ms.saturating_sub(now_ms().saturating_sub(last))).unwrap_or(0);

    if wait_ms >= deadline.remaining_ms() {
        return RateLimitOutcome::Declined { required_ms: wait_ms };
    }
    if wait_ms > 0 {
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    }

    // Replace the path rather than following a symlink when writing the timestamp.
    // Spacing remains best effort if the state directory cannot be written.
    let temporary = file.with_extension(format!("{}.tmp", rand::random::<u64>()));
    if let Ok(mut output) = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary) {
        if write!(output, "{}", now_ms()).is_ok() {
            let _ = fs::rename(&temporary, file);
        }
        let _ = fs::remove_file(temporary);
    }
    RateLimitOutcome::Ok { waited_ms: now_ms().saturating_sub(started_at) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("last-search");
        (dir, file)
    }

    fn options(file: &Path, timeout_ms: u64, interval_sec: u64) -> RateLimitOptions {
        RateLimitOptions {
            deadline: Deadline::new(timeout_ms),
            interval_sec: Some(interval_sec),
            file: Some(file.to_path_buf()),
        }
    }

    #[tokio::test]
    async fn does_not_follow_a_symlink_when_writing_the_timestamp() {
        let (dir, file) = sandbox();
        let target = dir.path().join("target");
        fs::write(&target, "sentinel").unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();

        assert!(reserve_search_slot(options(&file, 5000, 1)).await.is_ok());
        assert_eq!(fs::read_to_string(target).unwrap(), "sentinel");
        assert!(!fs::symlink_metadata(file).unwrap().file_type().is_symlink());
    }

    #[tokio::test]
    async fn lets_the_first_search_through_immediately() {
        let (_guard, file) = sandbox();
        assert!(reserve_search_slot(options(&file, 5000, 1)).await.is_ok());
    }

    #[tokio::test]
    async fn spaces_a_second_search_by_the_interval() {
        let (_guard, file) = sandbox();
        reserve_search_slot(options(&file, 5000, 1)).await;
        let started = std::time::Instant::now();
        reserve_search_slot(options(&file, 5000, 1)).await;
        assert!(started.elapsed().as_millis() >= 900);
    }

    /// The bug this guards: both callers read the same old timestamp, compute the
    /// same wait and then fire together, which is the burst the spacing exists to
    /// prevent. They have to queue, not race.
    #[tokio::test]
    async fn serializes_concurrent_callers_instead_of_releasing_them_together() {
        let (_guard, file) = sandbox();
        reserve_search_slot(options(&file, 10_000, 1)).await;
        let started = std::time::Instant::now();

        let mut finishes = futures::future::join_all((0..2).map(|_| {
            let file = file.clone();
            async move {
                reserve_search_slot(options(&file, 10_000, 1)).await;
                started.elapsed().as_millis()
            }
        }))
        .await;

        finishes.sort();
        assert!(finishes[0] >= 900, "first waited {}ms", finishes[0]);
        assert!(finishes[1] - finishes[0] >= 900, "second queued only {}ms later", finishes[1] - finishes[0]);
    }

    #[tokio::test]
    async fn declines_rather_than_overrunning_the_callers_deadline() {
        let (_guard, file) = sandbox();
        reserve_search_slot(options(&file, 5000, 10)).await;
        let outcome = reserve_search_slot(options(&file, 300, 10)).await;
        assert!(!outcome.is_ok());
    }
}
