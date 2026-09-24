use std::path::PathBuf;

/// Honour XDG so cached pages land beside every other tool's cache.
fn cache_home() -> PathBuf {
    let base = match std::env::var("XDG_CACHE_HOME") {
        Ok(dir) if dir.starts_with('/') => PathBuf::from(dir),
        _ => home_dir().join(".cache"),
    };
    base.join("search-headless")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

pub const SEARCH_LIMIT: usize = 10;
pub const FETCH_LIMIT: usize = 5;
pub const TIMEOUT_MS: u64 = 60_000;
pub const MAX_CHARS: usize = 50_000;
/// Concurrent page fetches for multi-URL work.
pub const CONCURRENCY: usize = 5;
/// A SERP goes stale quickly; an article does not.
pub const SEARCH_CACHE_TTL_MS: u64 = 15 * 60_000;
pub const FETCH_CACHE_TTL_MS: u64 = 6 * 60 * 60_000;
/// Largest response body the HTTP tier will read, in bytes.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Where cached search and fetch results live.
pub fn cache_dir() -> PathBuf {
    match std::env::var("SEARCH_HEADLESS_CACHE_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => cache_home(),
    }
}

/// Persistent Brave state, separate from disposable result caches.
pub fn search_state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SEARCH_HEADLESS_STATE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let base = match std::env::var("XDG_STATE_HOME") {
        Ok(dir) if dir.starts_with('/') => PathBuf::from(dir),
        _ => home_dir().join(".local/state"),
    };
    base.join("search-headless/brave")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_agent_friendly_defaults() {
        const _: () = {
            assert!(SEARCH_LIMIT == 10);
            assert!(FETCH_LIMIT == 5);
            assert!(TIMEOUT_MS == 60_000);
            assert!(MAX_CHARS == 50_000);
            assert!(CONCURRENCY == 5);
            // A SERP goes stale sooner than an article.
            assert!(SEARCH_CACHE_TTL_MS < FETCH_CACHE_TTL_MS);
        };
    }

    #[test]
    fn puts_the_cache_under_a_search_headless_directory() {
        // Mutating the environment would race the other tests in this binary, so
        // the override path is covered end to end by the integration suite.
        assert!(cache_dir().ends_with("search-headless"));
    }
}
