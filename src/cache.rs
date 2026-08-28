//! A tiny content-addressed disk cache.
//!
//! An agent asks for the same page across turns and reruns the same query while
//! exploring; every repeat otherwise costs a browser launch and, worse, one more
//! request against Brave's rate limit. Entries are written atomically so a
//! concurrent reader never sees a half-written file, and only successful results
//! are ever stored - a `blocked` SERP cached for fifteen minutes would be a bug,
//! not a speedup.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::config;

#[derive(Debug, Clone, Default)]
pub struct CacheOptions {
    /// Set false to bypass reads and writes entirely (`--no-cache`).
    pub enabled: Option<bool>,
    /// Freshness window in ms. Falls back to the namespace default.
    pub ttl_ms: Option<u64>,
    /// Override the cache root, mainly for tests.
    pub dir: Option<PathBuf>,
}

impl CacheOptions {
    /// Fill in a namespace's default TTL where the caller did not ask for one.
    pub fn with_default_ttl(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms.get_or_insert(ttl_ms);
        self
    }
}

#[derive(Serialize, serde::Deserialize)]
struct Envelope<T> {
    #[serde(rename = "expiresAt")]
    expires_at: u64,
    value: T,
}

/// One in this many writes also sweeps expired entries, to bound growth.
const PRUNE_ODDS: u32 = 20;

/// A `.tmp` older than this was left behind by a process that died mid-write.
const ORPHAN_TMP_MS: u64 = 60_000;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn enabled(options: &CacheOptions) -> bool {
    if options.enabled == Some(false) {
        return false;
    }
    std::env::var("SEARCH_HEADLESS_NO_CACHE").as_deref() != Ok("1")
}

fn namespace_dir(namespace: &str, options: &CacheOptions) -> PathBuf {
    options.dir.clone().unwrap_or_else(config::cache_dir).join(namespace)
}

fn entry_path(namespace: &str, key: &str, options: &CacheOptions) -> PathBuf {
    let digest = Sha256::digest(key.as_bytes());
    let mut hash = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        use std::fmt::Write;
        let _ = write!(hash, "{byte:02x}");
    }
    namespace_dir(namespace, options).join(format!("{hash}.json"))
}

/// Read a fresh entry, or `None` on a miss, an expiry or a damaged file.
pub fn cache_read<T: DeserializeOwned>(namespace: &str, key: &str, options: &CacheOptions) -> Option<T> {
    if !enabled(options) {
        return None;
    }
    let raw = fs::read_to_string(entry_path(namespace, key, options)).ok()?;
    let envelope: Envelope<T> = serde_json::from_str(&raw).ok()?;
    if envelope.expires_at <= now_ms() {
        return None;
    }
    Some(envelope.value)
}

/// Store a value. Failures are silent: a cache that cannot write is not an error.
pub fn cache_write<T: Serialize>(namespace: &str, key: &str, value: &T, options: &CacheOptions) {
    if !enabled(options) {
        return;
    }
    let ttl_ms = options.ttl_ms.unwrap_or(config::FETCH_CACHE_TTL_MS);
    if ttl_ms == 0 {
        return;
    }

    let path = entry_path(namespace, key, options);
    let dir = namespace_dir(namespace, options);
    if fs::create_dir_all(&dir).is_err() {
        return;
    }

    let envelope = Envelope { expires_at: now_ms() + ttl_ms, value };
    let Ok(encoded) = serde_json::to_vec(&envelope) else { return };

    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if fs::write(&tmp, &encoded).is_err() {
        let _ = fs::remove_file(&tmp);
        return;
    }
    if fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&tmp);
        return;
    }

    if rand::random_ratio(1, PRUNE_ODDS) {
        prune_cache(namespace, options);
    }
}

/// Delete expired entries in a namespace.
///
/// Expiry lives inside the file, so this parses each one; the sweep is rare
/// enough (see `PRUNE_ODDS`) that the cost does not land on a hot path.
pub fn prune_cache(namespace: &str, options: &CacheOptions) -> usize {
    let dir = namespace_dir(namespace, options);
    let Ok(entries) = fs::read_dir(&dir) else { return 0 };
    let mut removed = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
            if orphaned_tmp(&path) {
                let _ = fs::remove_file(&path);
                removed += 1;
            }
            continue;
        }
        if expired_or_damaged(&path) {
            let _ = fs::remove_file(&path);
            removed += 1;
        }
    }
    removed
}

fn orphaned_tmp(path: &Path) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|m| m.modified()) else { return true };
    modified.elapsed().map(|age| age.as_millis() as u64 > ORPHAN_TMP_MS).unwrap_or(false)
}

fn expired_or_damaged(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else { return true };
    match serde_json::from_str::<Envelope<serde_json::Value>>(&raw) {
        Ok(envelope) => envelope.expires_at <= now_ms(),
        Err(_) => true,
    }
}

/// Drop every entry in a namespace, or the whole cache when none is named.
pub fn clear_cache(namespace: Option<&str>, options: &CacheOptions) {
    let root = options.dir.clone().unwrap_or_else(config::cache_dir);
    let target = match namespace {
        Some(ns) => root.join(ns),
        None => root,
    };
    let _ = fs::remove_dir_all(target);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    fn sandbox() -> (tempfile::TempDir, CacheOptions) {
        let dir = tempfile::tempdir().expect("tempdir");
        let options = CacheOptions {
            enabled: None,
            ttl_ms: Some(60_000),
            dir: Some(dir.path().to_path_buf()),
        };
        (dir, options)
    }

    #[test]
    fn round_trips_a_value() {
        let (_guard, options) = sandbox();
        cache_write("ns", "key", &serde_json::json!({"hello": "world"}), &options);
        let got: serde_json::Value = cache_read("ns", "key", &options).expect("hit");
        assert_eq!(got, serde_json::json!({"hello": "world"}));
    }

    #[test]
    fn misses_on_an_unknown_key_and_on_another_namespace() {
        let (_guard, options) = sandbox();
        cache_write("ns", "key", &1u32, &options);
        assert!(cache_read::<u32>("ns", "other", &options).is_none());
        assert!(cache_read::<u32>("other", "key", &options).is_none());
    }

    #[test]
    fn treats_an_expired_entry_as_a_miss() {
        let (_guard, mut options) = sandbox();
        options.ttl_ms = Some(20);
        cache_write("ns", "key", &"stale", &options);
        sleep(Duration::from_millis(30));
        assert!(cache_read::<String>("ns", "key", &options).is_none());
    }

    #[test]
    fn bypasses_the_cache_entirely_when_disabled() {
        let (_guard, options) = sandbox();
        let off = CacheOptions { enabled: Some(false), ..options.clone() };

        cache_write("ns", "key", &"value", &off);
        assert!(cache_read::<String>("ns", "key", &options).is_none());

        cache_write("ns", "key", &"value", &options);
        assert!(cache_read::<String>("ns", "key", &off).is_none());
    }

    #[test]
    fn survives_a_corrupt_entry_instead_of_throwing() {
        let (_guard, options) = sandbox();
        cache_write("ns", "key", &"value", &options);
        let dir = options.dir.as_ref().unwrap().join("ns");
        let first = fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        fs::write(first, "{ not json").unwrap();
        assert!(cache_read::<String>("ns", "key", &options).is_none());
    }

    #[test]
    fn prunes_expired_entries_but_keeps_fresh_ones() {
        let (_guard, options) = sandbox();
        cache_write("ns", "fresh", &"keep", &options);
        // Long enough that the 1-in-PRUNE_ODDS sweep inside this very write cannot
        // find the entry already expired, short enough to expire before the sweep
        // under test.
        cache_write("ns", "stale", &"drop", &CacheOptions { ttl_ms: Some(50), ..options.clone() });
        sleep(Duration::from_millis(60));

        assert_eq!(prune_cache("ns", &options), 1);
        assert_eq!(cache_read::<String>("ns", "fresh", &options).as_deref(), Some("keep"));
    }

    #[test]
    fn clears_a_namespace() {
        let (_guard, options) = sandbox();
        cache_write("ns", "key", &"value", &options);
        clear_cache(Some("ns"), &options);
        assert!(cache_read::<String>("ns", "key", &options).is_none());
    }
}
