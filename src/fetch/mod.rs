pub mod http_extract;
pub mod obscura_extract;
pub mod url_rewrite;
pub mod url_validation;

use std::path::PathBuf;

use futures::StreamExt;

use crate::cache::{CacheOptions, cache_read, cache_write};
use crate::config;
use crate::types::{CommandStatus, Deadline, FetchResponse, FetchedContent, ToolError};

use http_extract::{HttpExtractDiagnostics, HttpExtractOptions, extract_via_http};
use obscura_extract::{ObscuraExtractOptions, extract_via_obscura};
use url_rewrite::rewrite_url;
use url_validation::{UrlValidationOptions, validate_public_http_url};

#[derive(Clone)]
pub struct FetchContentOptions {
    pub deadline: Deadline,
    pub max_chars: usize,
    pub allow_private_urls: bool,
    pub cache: CacheOptions,
    pub concurrency: usize,
    /// Override the obscura binary. `None` reads `OBSCURA_BIN`, then PATH.
    pub obscura_bin: Option<PathBuf>,
}

impl FetchContentOptions {
    pub fn new(deadline: Deadline) -> Self {
        Self {
            deadline,
            max_chars: config::MAX_CHARS,
            allow_private_urls: false,
            cache: CacheOptions::default().with_default_ttl(config::FETCH_CACHE_TTL_MS),
            concurrency: config::CONCURRENCY,
            obscura_bin: None,
        }
    }
}

/// Below this, a script-bearing page is treated as possibly unrendered.
const MIN_CONTENT_CHARS: usize = 500;

const CACHE_NAMESPACE: &str = "fetch";

/// HTTP-tier failures the browser tier cannot possibly fix.
const TERMINAL_HTTP_CODES: [&str; 5] =
    ["FETCH_NOT_HTML", "RESPONSE_TOO_LARGE", "TOO_MANY_REDIRECTS", "PRIVATE_URL_BLOCKED", "INVALID_URL"];

/// Response codes where a real browser plausibly gets a different answer.
///
/// These are the ones that mean "not you", not "not here": a bot wall, a rate
/// limit, a login gate. A 404 or a 500 is the same page in any client, and
/// rendering it just returns the error page's own text as if it were the article.
const ESCALATE_STATUSES: [u16; 6] = [401, 403, 405, 406, 429, 451];

/// Should the browser tier see this HTTP-tier failure at all?
fn worth_rendering(errors: &[ToolError]) -> bool {
    for error in errors {
        if TERMINAL_HTTP_CODES.contains(&error.code.as_str()) {
            return false;
        }
        if error.code == "FETCH_HTTP_ERROR" {
            let status = error.details.as_ref().and_then(|d| d.get("status")).and_then(|s| s.as_u64()).unwrap_or(0);
            if !ESCALATE_STATUSES.contains(&(status as u16)) {
                return false;
            }
        }
    }
    true
}

/// Decide whether an exhausted set of tiers should read as a timeout.
///
/// Checking the clock alone is racy: with a very short deadline every tier can
/// decline to start and still leave a millisecond on the clock.
fn timed_out(deadline: &Deadline, errors: &[ToolError]) -> bool {
    if deadline.expired() {
        return true;
    }
    !errors.is_empty() && errors.iter().all(|error| error.code == "FETCH_TIMEOUT")
}

/// Is the HTTP result good enough to skip the browser?
///
/// A JavaScript shell still yields text - nav links, a "Loading…" placeholder, a
/// cookie banner - and Readability happily returns it, so length alone would let
/// an unrendered page through as success. Escalate only when the page carries
/// script that could have produced more: a script-free page has nothing further
/// to render, however short it is.
fn http_result_is_trustworthy(result: &FetchedContent, diagnostics: &HttpExtractDiagnostics) -> bool {
    result.text_length() >= MIN_CONTENT_CHARS || diagnostics.script_count == Some(0)
}

/// Truncation happens per request, so the cap belongs in the key.
fn cache_key(url: &str, max_chars: usize) -> String {
    format!("{max_chars}:{url}")
}

/// Extract page content with a two-tier fallback strategy:
/// 0. Cache, then URL rewrite (e.g. Reddit: www → old)
/// 1. HTTP + Readability (plain HTTP, no browser)
/// 2. Obscura (headless browser, for JS-rendered pages)
///
/// HTTP goes first because it is both cheaper and higher quality: Readability
/// yields separated paragraphs plus byline/excerpt/publishedTime metadata, while
/// obscura has no layout engine and so returns `textContent` with no metadata.
/// Obscura handles what HTTP cannot - pages whose content is rendered by script -
/// and a thin HTTP result on a scripted page is re-checked against it rather than
/// returned as success.
pub async fn fetch_content(url: &str, options: &FetchContentOptions) -> FetchedContent {
    let validation_options = UrlValidationOptions { allow_private_urls: options.allow_private_urls };
    if let Err(error) = validate_public_http_url(url, validation_options) {
        return FetchedContent::failure(url, CommandStatus::Error, error);
    }

    let key = cache_key(url, options.max_chars);
    if let Some(mut cached) = cache_read::<FetchedContent>(CACHE_NAMESPACE, &key, &options.cache) {
        cached.cached = Some(true);
        return cached;
    }

    // Tier 0: domain-specific URL rewrites.
    let target = rewrite_url(url);
    let mut errors: Vec<ToolError> = Vec::new();

    // Tier 1: HTTP + Readability (cheap, best text and metadata).
    let http_options = HttpExtractOptions {
        allow_private_urls: options.allow_private_urls,
        ..HttpExtractOptions::new(options.deadline, options.max_chars)
    };
    let mut diagnostics = HttpExtractDiagnostics::default();
    let http_result = extract_via_http(&target, &http_options, &mut diagnostics).await;

    let http_ok = http_result.status == CommandStatus::Ok;
    if http_ok && http_result_is_trustworthy(&http_result, &diagnostics) {
        return store(url, options, http_result);
    }
    if !http_ok {
        errors.extend(http_result.errors.iter().cloned());
    }

    // A PDF stays a PDF and a 404 stays a 404; only failures a browser might see
    // differently are worth the cost of rendering.
    let terminal = !worth_rendering(&http_result.errors);

    // Tier 2: Obscura (renders JavaScript).
    if !terminal && options.deadline.remaining_ms() > 1000 {
        let obscura_options = ObscuraExtractOptions {
            deadline: options.deadline,
            max_chars: options.max_chars,
            allow_private_urls: options.allow_private_urls,
            obscura_bin: options.obscura_bin.clone(),
        };
        let obscura_result = extract_via_obscura(&target, &obscura_options).await;

        // Rendering can reveal content HTTP could not see; keep whichever saw more.
        if obscura_result.status == CommandStatus::Ok {
            if http_ok && http_result.text_length() >= obscura_result.text_length() {
                return store(url, options, http_result);
            }
            return store(url, options, obscura_result);
        }
        errors.extend(obscura_result.errors.iter().cloned());
    }

    // Obscura could not improve on a usable HTTP result - return it rather than fail.
    if http_ok {
        return store(url, options, http_result);
    }

    // All tiers failed - return the combined errors.
    let status = if timed_out(&options.deadline, &errors) { CommandStatus::Timeout } else { CommandStatus::Error };
    FetchedContent::failures(url, status, errors)
}

/// Cache a successful result under the URL the caller asked for, and return it.
fn store(url: &str, options: &FetchContentOptions, result: FetchedContent) -> FetchedContent {
    cache_write(CACHE_NAMESPACE, &cache_key(url, options.max_chars), &result, &options.cache);
    result
}

/// Fetch several URLs concurrently, preserving input order.
///
/// Both tiers parallelize well: HTTP is I/O-bound, and each obscura fetch is its
/// own process.
pub async fn fetch_many_content(urls: &[String], options: &FetchContentOptions) -> Vec<FetchedContent> {
    let concurrency = options.concurrency.max(1);
    futures::stream::iter(urls.iter().map(|url| fetch_content(url, options)))
        .buffered(concurrency)
        .collect()
        .await
}

/// Fetch a batch and roll the outcomes into one response.
///
/// `partial` means at least one URL worked: an agent asking for five pages and
/// getting four should still get the four, and a non-zero exit only when nothing
/// came back at all.
pub async fn fetch_content_batch(urls: &[String], options: &FetchContentOptions) -> FetchResponse {
    let results = fetch_many_content(urls, options).await;
    let ok_count = results.iter().filter(|r| r.status == CommandStatus::Ok).count();

    let status = if ok_count == results.len() {
        CommandStatus::Ok
    } else if ok_count > 0 {
        CommandStatus::Partial
    } else if results.iter().all(|r| r.status == CommandStatus::Timeout) {
        CommandStatus::Timeout
    } else {
        CommandStatus::Error
    };

    let errors = if ok_count == results.len() {
        Vec::new()
    } else {
        results.iter().flat_map(|r| r.errors.iter().cloned()).collect()
    };

    FetchResponse { status, results, errors }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, TestServer};
    use std::os::unix::fs::PermissionsExt;

    /// A binary that does not exist, so any escalation to the browser tier is a
    /// visible failure rather than a silent success.
    fn no_browser() -> Option<PathBuf> {
        Some(PathBuf::from("/nonexistent/obscura"))
    }

    /// Stand-in obscura returning the text a real render would have produced.
    fn fake_obscura_returning(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("fake-obscura");
        let payload = serde_json::json!({ "title": "Rendered", "url": "https://rendered.test/", "text": text });
        let script = format!("#!/usr/bin/env bash\nprintf '%s' {:?}\n", payload.to_string());
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, bin)
    }

    fn options(obscura_bin: Option<PathBuf>) -> FetchContentOptions {
        FetchContentOptions {
            max_chars: 50_000,
            allow_private_urls: true,
            cache: CacheOptions { enabled: Some(false), ..Default::default() },
            obscura_bin,
            ..FetchContentOptions::new(Deadline::new(20_000))
        }
    }

    async fn serve_html(body: &'static str) -> TestServer {
        testing::serve(move |_| testing::html(body)).await
    }

    fn page(server: &TestServer) -> String {
        format!("{}/page", server.base_url)
    }

    #[tokio::test]
    async fn serves_a_script_free_page_straight_from_http_without_starting_a_browser() {
        let server = serve_html(
            "<!doctype html><html><head><title>Short</title></head><body><h1>Short</h1><p>A brief but complete page.</p></body></html>",
        )
        .await;

        let result = fetch_content(&page(&server), &options(no_browser())).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.method, Some(crate::types::FetchMethod::Http));
    }

    #[tokio::test]
    async fn escalates_a_javascript_shell_to_the_browser_instead_of_returning_the_placeholder() {
        // Regression: HTTP-first previously returned the nav + "Loading…" + cookie
        // banner of an unrendered SPA as a successful extraction.
        let rendered = "The actual rendered article body that only exists after scripts run. ".repeat(12);
        let (_guard, bin) = fake_obscura_returning(&rendered);
        let server = serve_html(
            r#"<!doctype html><html><head><title>Dashboard</title></head><body>
            <nav>Home About Contact</nav>
            <div id="root"><p>Loading your dashboard, please wait...</p></div>
            <div class="cookie">We use cookies to improve your experience. Accept all cookies to continue.</div>
            <script>document.getElementById('root').innerHTML = '<article>real</article>';</script>
            </body></html>"#,
        )
        .await;

        let result = fetch_content(&page(&server), &options(Some(bin))).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.method, Some(crate::types::FetchMethod::Browser));
        let text = result.text.unwrap();
        assert!(text.contains("actual rendered article body"));
        assert!(!text.contains("Loading your dashboard"));
    }

    #[tokio::test]
    async fn keeps_the_http_result_when_the_browser_cannot_improve_on_it() {
        let (_guard, bin) = fake_obscura_returning("tiny");
        let server = serve_html(
            "<!doctype html><html><head><title>Shell</title></head><body><p>Some short text on a scripted page.</p><script>void 0;</script></body></html>",
        )
        .await;

        let result = fetch_content(&page(&server), &options(Some(bin))).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.method, Some(crate::types::FetchMethod::Http));
    }

    #[tokio::test]
    async fn accepts_a_long_http_result_on_a_scripted_page_without_escalating() {
        let server = testing::serve(|_| {
            let body = "This is a full article paragraph with plenty of real content. ".repeat(20);
            testing::html(format!(
                "<!doctype html><html><head><title>Article</title></head><body><article><h1>Article</h1><p>{body}</p></article><script>void 0;</script></body></html>"
            ))
        })
        .await;

        let result = fetch_content(&page(&server), &options(no_browser())).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.method, Some(crate::types::FetchMethod::Http));
    }

    #[tokio::test]
    async fn fetches_many_urls_concurrently_preserving_order() {
        let mut servers = Vec::new();
        let mut urls = Vec::new();
        for name in ["Alpha", "Beta", "Gamma"] {
            let server = testing::serve(move |_| {
                testing::html(format!(
                    "<!doctype html><html><head><title>{name}</title></head><body><h1>{name}</h1><p>Body of {name}.</p></body></html>"
                ))
            })
            .await;
            urls.push(page(&server));
            servers.push(server);
        }

        let mut options = options(no_browser());
        options.concurrency = 3;
        let results = fetch_many_content(&urls, &options).await;

        assert_eq!(results.iter().map(|r| r.status).collect::<Vec<_>>(), vec![CommandStatus::Ok; 3]);
        assert_eq!(
            results.iter().map(|r| r.title.clone().unwrap_or_default()).collect::<Vec<_>>(),
            vec!["Alpha", "Beta", "Gamma"]
        );
    }

    #[tokio::test]
    async fn keeps_a_usable_http_result_when_the_browser_tier_fails_outright() {
        // Regression: a thin-but-real page was discarded entirely when the escalation
        // it triggered could not run, turning content into an error.
        let server = serve_html(
            "<!doctype html><html><head><title>Shell</title></head><body><p>Loading...</p><script>void 0;</script></body></html>",
        )
        .await;

        let result = fetch_content(&page(&server), &options(no_browser())).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.text.unwrap().contains("Loading..."));
    }

    #[tokio::test]
    async fn does_not_start_a_browser_for_a_content_type_rendering_cannot_fix() {
        let server = testing::serve(|_| testing::with_type("%PDF-1.7", "application/pdf")).await;

        let result = fetch_content(&format!("{}/doc.pdf", server.base_url), &options(no_browser())).await;
        assert_eq!(result.status, CommandStatus::Error);
        assert_eq!(result.errors[0].code, "FETCH_NOT_HTML");
        // No browser attempt was appended.
        assert_eq!(result.errors.len(), 1);
    }

    #[tokio::test]
    async fn serves_a_repeat_fetch_from_cache_without_touching_the_network() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        let server = testing::serve(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            testing::html(
                "<!doctype html><html><head><title>Cached</title></head><body><p>Stable content.</p></body></html>",
            )
        })
        .await;

        let dir = tempfile::tempdir().unwrap();
        let mut options = options(no_browser());
        options.cache = CacheOptions {
            enabled: Some(true),
            ttl_ms: Some(60_000),
            dir: Some(dir.path().to_path_buf()),
        };

        let first = fetch_content(&page(&server), &options).await;
        let second = fetch_content(&page(&server), &options).await;

        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(first.cached, None);
        assert_eq!(second.cached, Some(true));
        assert_eq!(second.text, first.text);
    }

    #[tokio::test]
    async fn does_not_render_a_404_page_and_pass_its_text_off_as_the_article() {
        let (_guard, bin) = fake_obscura_returning("Sorry, that page could not be found. Try our homepage.");
        let server =
            testing::serve(|_| testing::status(404, "<html><body><h1>Not found</h1></body></html>", "text/html")).await;

        let result = fetch_content(&format!("{}/missing", server.base_url), &options(Some(bin))).await;
        assert_eq!(result.status, CommandStatus::Error);
        assert_eq!(result.errors[0].code, "FETCH_HTTP_ERROR");
    }

    #[tokio::test]
    async fn still_escalates_a_bot_wall_which_a_browser_may_be_served_differently() {
        let rendered = "The article body a real browser is allowed to see. ".repeat(10);
        let (_guard, bin) = fake_obscura_returning(&rendered);
        let server = testing::serve(|_| testing::status(403, "<html><body>Forbidden</body></html>", "text/html")).await;

        let result = fetch_content(&format!("{}/gated", server.base_url), &options(Some(bin))).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.method, Some(crate::types::FetchMethod::Browser));
    }
}
