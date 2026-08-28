use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::cache::{CacheOptions, cache_read, cache_write};
use crate::config;
use crate::fetch::{FetchContentOptions, fetch_many_content};
use crate::obscura::{ObscuraFetchArgs, obscura_fetch, parse_obscura_json};
use crate::rate_limit::{RateLimitOptions, RateLimitOutcome, reserve_search_slot};
use crate::types::{CommandStatus, Deadline, SearchEngine, SearchResponse, SearchResult, ToolError};

use super::parse_brave::{build_brave_search_url, detect_brave_block, parse_brave_next_url, parse_brave_results};

const CACHE_NAMESPACE: &str = "search";

/// A page load that failed for a reason other than a challenge is retried once.
const LOAD_ATTEMPTS: usize = 2;
const RETRY_BACKOFF_MS: u64 = 1500;

pub struct SearchBraveOptions {
    pub limit: usize,
    pub fetch: bool,
    pub fetch_limit: usize,
    pub deadline: Deadline,
    pub allow_private_urls: bool,
    pub fixture_path: Option<PathBuf>,
    pub cache: CacheOptions,
    pub concurrency: usize,
    /// Override the obscura binary. `None` reads `OBSCURA_BIN`, then PATH.
    pub obscura_bin: Option<PathBuf>,
}

impl SearchBraveOptions {
    pub fn new(deadline: Deadline) -> Self {
        Self {
            limit: config::SEARCH_LIMIT,
            fetch: false,
            fetch_limit: config::FETCH_LIMIT,
            deadline,
            allow_private_urls: false,
            fixture_path: None,
            cache: CacheOptions::default().with_default_ttl(config::SEARCH_CACHE_TTL_MS),
            concurrency: config::CONCURRENCY,
            obscura_bin: None,
        }
    }
}

/// Hand the rendered page back for parsing here rather than picking results
/// apart in the page.
///
/// Nothing is clicked, and the parse itself stays in Rust where the fixture
/// tests can reach it. Only inert weight is dropped before the HTML crosses the
/// pipe - a Brave SERP carries far more inline script than markup, and shipping
/// it costs more than the parse does. The footer stays, because Brave's "Next"
/// link lives there; `noscript` stays, because a JavaScript-required notice is
/// one of the interstitials that has to be detected.
pub const READ_SERP_SCRIPT: &str = concat!(
    "(() => {",
    "  document.querySelectorAll('script,style,svg,link,iframe,template,picture,source').forEach(el => el.remove());",
    "  return JSON.stringify({",
    "    html: document.documentElement.outerHTML,",
    "    text: document.body ? document.body.textContent || '' : '',",
    "    url: window.location.href,",
    "  });",
    "})()"
);

#[derive(Debug, Clone, Deserialize)]
pub struct SerpPage {
    pub html: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub url: String,
}

/// Load one Brave SERP through obscura, retrying a transient failure once.
///
/// A crashed or missing browser is not a bot challenge, and reporting it as one
/// sends an agent off to solve the wrong problem - so the two stay distinct all
/// the way out to the caller. A challenge is never retried: it is reported.
async fn load_serp(url: &str, deadline: &Deadline, bin: Option<&PathBuf>) -> Result<SerpPage, ToolError> {
    let mut last = ToolError::new("OBSCURA_SEARCH_FAILED", "obscura produced no output.", true);

    for attempt in 1..=LOAD_ATTEMPTS {
        if deadline.remaining_ms() <= 1000 {
            return Err(ToolError::new(
                "FETCH_TIMEOUT",
                "Overall timeout expired before the results page loaded.",
                true,
            ));
        }

        let mut args = ObscuraFetchArgs::new(url, *deadline).eval(READ_SERP_SCRIPT).stealth(true);
        args.bin = bin.cloned();
        let result = obscura_fetch(&args).await;

        if result.ok {
            match parse_obscura_json::<SerpPage>(&result.stdout) {
                Some(page) => return Ok(page),
                None => {
                    last = ToolError::new("OBSCURA_SEARCH_FAILED", "obscura returned no readable results page.", true);
                }
            }
        } else if deadline.expired() {
            return Err(ToolError::new(
                "FETCH_TIMEOUT",
                "Overall timeout expired while loading the results page.",
                true,
            ));
        } else {
            let message = if result.stderr.trim().is_empty() {
                format!("obscura exited with code {}.", result.exit_code)
            } else {
                result.stderr.trim().to_string()
            };
            last = ToolError::new("OBSCURA_SEARCH_FAILED", message, true);
        }

        if attempt < LOAD_ATTEMPTS && deadline.remaining_ms() > RETRY_BACKOFF_MS + 1000 {
            tokio::time::sleep(Duration::from_millis(RETRY_BACKOFF_MS)).await;
        }
    }

    Err(last)
}

fn response(
    query: &str,
    status: CommandStatus,
    results: Vec<SearchResult>,
    errors: Vec<ToolError>,
) -> SearchResponse {
    SearchResponse {
        query: query.to_string(),
        engine: SearchEngine::Brave,
        status,
        results,
        cached: None,
        errors,
    }
}

fn blocked_response(query: &str, message: &str, results: Vec<SearchResult>) -> SearchResponse {
    response(query, CommandStatus::Blocked, results, vec![ToolError::new("BRAVE_CHALLENGE", message, true)])
}

/// Read SERP pages from a fixture instead of the network, for tests.
fn load_fixture_serp(fixture_path: &PathBuf) -> Vec<SerpPage> {
    let mut pages = Vec::new();
    if let Ok(html) = std::fs::read_to_string(fixture_path) {
        pages.push(SerpPage {
            text: html.clone(),
            html,
            url: "https://search.brave.com/search".to_string(),
        });
    }
    if let Ok(second) = std::env::var("SEARCH_HEADLESS_BRAVE_FIXTURE_2")
        && let Ok(html) = std::fs::read_to_string(&second)
    {
        pages.push(SerpPage {
            text: html.clone(),
            html,
            url: "https://search.brave.com/search?offset=1".to_string(),
        });
    }
    pages
}

/// Resolve the fixture path, honouring the environment override.
pub fn fixture_path(options: &SearchBraveOptions) -> Option<PathBuf> {
    options.fixture_path.clone().or_else(|| match std::env::var("SEARCH_HEADLESS_BRAVE_FIXTURE") {
        Ok(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => None,
    })
}

/// Search Brave.
///
/// Obscura is the only engine. If Brave serves a challenge instead of results,
/// that is reported as `status: "blocked"` - never as an empty success, and
/// never worked around.
pub async fn search_brave(query: &str, options: &SearchBraveOptions) -> SearchResponse {
    let key = format!("{}:{}", options.limit, query);
    let fixture = fixture_path(options);
    let fixture_pages = fixture.as_ref().map(load_fixture_serp);
    let mut errors: Vec<ToolError> = Vec::new();

    // A repeated query is answered from disk, which also spares the rate limit.
    let cached = match fixture_pages {
        Some(_) => None,
        None => cache_read::<SearchResponse>(CACHE_NAMESPACE, &key, &options.cache),
    };

    let base = match cached.clone() {
        Some(cached) => cached,
        None => collect(query, options, fixture_pages.as_deref(), &mut errors).await,
    };
    if matches!(base.status, CommandStatus::Blocked | CommandStatus::Error | CommandStatus::Timeout) {
        return base;
    }

    let mut result = base;
    if cached.is_some() {
        result.cached = Some(true);
    } else {
        cache_write(CACHE_NAMESPACE, &key, &result, &options.cache);
    }

    if !options.fetch || result.results.is_empty() {
        return result;
    }

    // Content fetches have their own cache and their own failure modes; a bad
    // fetch downgrades an otherwise good search to `partial`, never to `blocked`.
    let n = options.fetch_limit.min(result.results.len());
    let targets: Vec<String> = result.results[..n].iter().map(|r| r.url.clone()).collect();

    let fetch_options = FetchContentOptions {
        allow_private_urls: options.allow_private_urls,
        cache: options.cache.clone().with_default_ttl(config::FETCH_CACHE_TTL_MS),
        concurrency: options.concurrency,
        obscura_bin: options.obscura_bin.clone(),
        ..FetchContentOptions::new(options.deadline)
    };
    let fetched = fetch_many_content(&targets, &fetch_options).await;

    let mut fetch_failed = false;
    for (target, content) in result.results.iter_mut().zip(fetched) {
        if content.status != CommandStatus::Ok {
            fetch_failed = true;
        }
        target.fetched = Some(content);
    }

    if fetch_failed || !result.errors.is_empty() {
        result.status = CommandStatus::Partial;
    }
    result
}

/// Load and parse SERP pages until the limit is met or Brave stops cooperating.
async fn collect(
    query: &str,
    options: &SearchBraveOptions,
    fixture_pages: Option<&[SerpPage]>,
    errors: &mut Vec<ToolError>,
) -> SearchResponse {
    let first = match fixture_pages {
        Some(pages) => match pages.first() {
            Some(page) => page.clone(),
            None => return blocked_response(query, "Brave Search did not return a readable results page.", vec![]),
        },
        None => {
            let slot = reserve_search_slot(RateLimitOptions {
                deadline: options.deadline,
                interval_sec: None,
                file: None,
            })
            .await;
            if let RateLimitOutcome::Declined { required_ms } = slot {
                return response(
                    query,
                    CommandStatus::Timeout,
                    vec![],
                    vec![ToolError::new(
                        "FETCH_TIMEOUT",
                        format!(
                            "Rate limiting needs another {}s, which is longer than the remaining timeout.",
                            required_ms.div_ceil(1000)
                        ),
                        true,
                    )],
                );
            }

            match load_serp(&build_brave_search_url(query, None), &options.deadline, options.obscura_bin.as_ref()).await
            {
                Ok(page) => page,
                Err(error) => {
                    let status = if error.code == "FETCH_TIMEOUT" { CommandStatus::Timeout } else { CommandStatus::Error };
                    return response(query, status, vec![], vec![error]);
                }
            }
        }
    };

    if detect_brave_block(&first.html, &first.text) {
        return blocked_response(query, "Brave Search served a bot challenge instead of results.", vec![]);
    }

    let mut all_results: Vec<SearchResult> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    add(&mut all_results, &mut seen, &first, options.limit);

    // Follow pagination until the limit is met.
    let mut next_url = parse_brave_next_url(&first.html);
    let mut fixture_index = 1;

    while all_results.len() < options.limit && options.deadline.remaining_ms() > 2000 {
        let serp = match fixture_pages {
            Some(pages) => {
                let page = pages.get(fixture_index).cloned();
                fixture_index += 1;
                page
            }
            None => match &next_url {
                Some(url) => {
                    let slot = reserve_search_slot(RateLimitOptions {
                        deadline: options.deadline,
                        interval_sec: None,
                        file: None,
                    })
                    .await;
                    if !slot.is_ok() {
                        break;
                    }
                    match load_serp(url, &options.deadline, options.obscura_bin.as_ref()).await {
                        Ok(page) => Some(page),
                        Err(error) => {
                            errors.push(error);
                            break;
                        }
                    }
                }
                None => None,
            },
        };
        let Some(serp) = serp else { break };

        if detect_brave_block(&serp.html, &serp.text) {
            // A challenge outranks a partial page: a later --fetch problem must
            // not downgrade "blocked" to "partial" and turn a non-zero exit into
            // success.
            return blocked_response(query, "Brave Search served a bot challenge while paginating.", all_results);
        }

        let before = all_results.len();
        add(&mut all_results, &mut seen, &serp, options.limit);
        next_url = parse_brave_next_url(&serp.html);
        if all_results.len() == before {
            break; // no progress
        }
    }

    let status = if errors.is_empty() { CommandStatus::Ok } else { CommandStatus::Partial };
    response(query, status, all_results, std::mem::take(errors))
}

fn add(all: &mut Vec<SearchResult>, seen: &mut HashSet<String>, serp: &SerpPage, limit: usize) {
    for mut result in parse_brave_results(&serp.html, limit, 1) {
        if all.len() >= limit {
            break;
        }
        if !seen.insert(canonical_url(&result.url)) {
            continue;
        }
        result.rank = all.len() + 1;
        all.push(result);
    }
}

/// Key used to spot the same page twice across SERP pages.
///
/// Brave repeats a result between pages with a different tracking suffix often
/// enough that exact string matching lets duplicates through.
pub fn canonical_url(input: &str) -> String {
    let Ok(mut url) = Url::parse(input) else {
        return input.to_string();
    };
    url.set_fragment(None);

    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| !is_tracking_param(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(&kept);
    }

    let host = url.host_str().unwrap_or("").trim_start_matches("www.").to_string();
    let path = url.path().trim_end_matches('/').to_string();
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();

    format!("{}://{}{}{}", url.scheme(), host, path, query).to_lowercase()
}

fn is_tracking_param(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("utm_")
        || key.starts_with("ref_")
        || key.starts_with("mc_cid")
        || key.starts_with("mc_eid")
        || matches!(key.as_str(), "ref" | "fbclid" | "gclid" | "igshid" | "spm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalises_urls_so_the_same_result_is_not_ranked_twice() {
        let a = canonical_url("https://www.Example.com/post/?utm_source=brave&id=7#top");
        let b = canonical_url("https://example.com/post?id=7");
        assert_eq!(a, b);
    }

    #[test]
    fn keeps_genuinely_different_urls_apart() {
        assert_ne!(canonical_url("https://example.com/a"), canonical_url("https://example.com/b"));
        assert_ne!(canonical_url("https://example.com/a?page=1"), canonical_url("https://example.com/a?page=2"));
    }

    #[test]
    fn passes_an_unparseable_url_through_unchanged() {
        assert_eq!(canonical_url("not a url"), "not a url");
    }

    #[test]
    fn strips_every_tracking_parameter_it_knows() {
        let noisy = "https://example.com/a?utm_source=x&ref=y&ref_src=z&fbclid=1&gclid=2&mc_cid=3&igshid=4&spm=5&keep=me";
        assert_eq!(canonical_url(noisy), "https://example.com/a?keep=me");
    }
}
