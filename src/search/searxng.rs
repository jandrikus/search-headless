//! Authenticated searches against an explicitly configured SearXNG instance.
use std::collections::HashSet;
use std::time::Duration;

use dom_query::Document;
use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::cache::{CacheOptions, cache_read, cache_write};
use crate::config;
use crate::fetch::{FetchContentOptions, fetch_many_content};
use crate::types::{CommandStatus, Deadline, SearchEngine, SearchResponse, SearchResult, ToolError};

pub struct SearchSearxngOptions {
    pub instance_url: Option<String>,
    pub limit: usize,
    pub paginate: bool,
    pub fetch: bool,
    pub fetch_limit: usize,
    pub deadline: Deadline,
    pub cache: CacheOptions,
    pub concurrency: usize,
    pub allow_private_urls: bool,
}

impl SearchSearxngOptions {
    pub fn new(deadline: Deadline) -> Self {
        Self {
            instance_url: None,
            limit: config::SEARCH_LIMIT,
            paginate: false,
            fetch: false,
            fetch_limit: config::FETCH_LIMIT,
            deadline,
            cache: CacheOptions::default().with_default_ttl(config::SEARCH_CACHE_TTL_MS),
            concurrency: config::CONCURRENCY,
            allow_private_urls: false,
        }
    }
}

#[derive(Deserialize)]
struct Page {
    results: Vec<Entry>,
    #[serde(default)]
    unresponsive_engines: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct Entry {
    url: String,
    title: String,
    #[serde(default)]
    content: Option<String>,
}

// Deliberately not Debug: neither credential belongs in diagnostics.
struct Credentials {
    user: String,
    pass: String,
}

impl Credentials {
    fn from_env() -> Result<Self, ToolError> {
        let user = std::env::var("SEARXNG_USER").unwrap_or_default();
        let pass = std::env::var("SEARXNG_PASS").unwrap_or_default();
        if user.is_empty() || pass.is_empty() {
            return Err(ToolError::new(
                "SEARXNG_AUTH_MISSING",
                "Set SEARXNG_USER and SEARXNG_PASS in the environment that starts Pi. Credentials are not read from settings.",
                false,
            ));
        }
        if user.contains(':') {
            return Err(ToolError::new(
                "SEARXNG_AUTH_INVALID",
                "SEARXNG_USER must not contain a colon for HTTP Basic authentication.",
                false,
            ));
        }
        Ok(Self { user, pass })
    }
}

fn endpoint(options: &SearchSearxngOptions) -> Result<Url, ToolError> {
    let invalid = || {
        ToolError::new(
            "SEARXNG_CONFIG_ERROR",
            "Configure a SearXNG base URL using --searxng-url or SEARXNG_URL. Use HTTPS without credentials, query parameters, or fragments.",
            false,
        )
    };
    let value = options.instance_url.clone().or_else(|| std::env::var("SEARXNG_URL").ok()).ok_or_else(invalid)?;
    let mut url = Url::parse(&value).map_err(|_| invalid())?;
    let test_http = options.allow_private_urls
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if (url.scheme() != "https" && !test_http)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    url.set_path(&format!("{}/search", url.path().trim_end_matches('/')));
    Ok(url)
}

fn failure(result: &mut SearchResponse, error: ToolError) {
    result.status = if !result.results.is_empty() {
        CommandStatus::Partial
    } else if error.code == "SEARXNG_TIMEOUT" {
        CommandStatus::Timeout
    } else {
        CommandStatus::Error
    };
    result.errors.push(error);
}

pub async fn search_searxng(query: &str, options: &SearchSearxngOptions) -> SearchResponse {
    let mut result = SearchResponse {
        query: query.to_string(),
        engine: SearchEngine::Searxng,
        status: CommandStatus::Ok,
        results: vec![],
        cached: None,
        errors: vec![],
    };
    let endpoint = match endpoint(options) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            failure(&mut result, error);
            return result;
        }
    };
    let credentials = match Credentials::from_env() {
        Ok(credentials) => credentials,
        Err(error) => {
            failure(&mut result, error);
            return result;
        }
    };
    // The cache hashes this key; credentials are never written as file contents.
    // Different instances and users must not share cached searches.
    let key =
        serde_json::json!([endpoint.as_str(), credentials.user, options.limit, options.paginate, query]).to_string();
    if let Some(mut cached) = cache_read::<SearchResponse>("searxng", &key, &options.cache) {
        cached.cached = Some(true);
        result = cached;
    } else {
        let client = match Client::builder().redirect(reqwest::redirect::Policy::none()).build() {
            Ok(client) => client,
            Err(_) => {
                failure(
                    &mut result,
                    ToolError::new("SEARXNG_HTTP_ERROR", "Could not initialize the SearXNG HTTP client.", false),
                );
                return result;
            }
        };
        collect(&client, &endpoint, &credentials, options, &mut result).await;
        if result.status == CommandStatus::Ok {
            cache_write("searxng", &key, &result, &options.cache);
        }
    }
    if options.fetch && !result.results.is_empty() {
        let urls: Vec<_> = result.results.iter().take(options.fetch_limit).map(|r| r.url.clone()).collect();
        let fetch_options = FetchContentOptions {
            allow_private_urls: options.allow_private_urls,
            cache: options.cache.clone().with_default_ttl(config::FETCH_CACHE_TTL_MS),
            concurrency: options.concurrency,
            ..FetchContentOptions::new(options.deadline)
        };
        // Fetch uses its own client. The SearXNG Authorization header is never
        // sent to result URLs or Obscura.
        for (entry, fetched) in result.results.iter_mut().zip(fetch_many_content(&urls, &fetch_options).await) {
            if fetched.status != CommandStatus::Ok {
                result.status = CommandStatus::Partial;
            }
            entry.fetched = Some(fetched);
        }
    }
    result
}

async fn collect(
    client: &Client,
    endpoint: &Url,
    credentials: &Credentials,
    options: &SearchSearxngOptions,
    result: &mut SearchResponse,
) {
    let mut seen = HashSet::new();
    let mut page_number = 1usize;
    loop {
        let page =
            match request_page(client, endpoint, credentials, &result.query, page_number, &options.deadline).await {
                Ok(page) => page,
                Err(error) => {
                    failure(result, error);
                    return;
                }
            };
        let before = result.results.len();
        let nonempty = !page.results.is_empty();
        let mut invalid = 0;
        for entry in page.results {
            if result.results.len() >= options.limit {
                break;
            }
            let Ok(mut url) = Url::parse(&entry.url) else {
                invalid += 1;
                continue;
            };
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                invalid += 1;
                continue;
            }
            url.set_fragment(None);
            let title = plain_text(&entry.title);
            if title.is_empty() {
                invalid += 1;
                continue;
            }
            if !seen.insert(url.to_string()) {
                continue;
            }
            let snippet = entry.content.map(|content| plain_text(&content)).filter(|text| !text.is_empty());
            result.results.push(SearchResult {
                rank: result.results.len() + 1,
                title,
                url: url.to_string(),
                display_url: url.host_str().map(str::to_string),
                snippet,
                fetched: None,
            });
        }
        if invalid > 0 {
            failure(
                result,
                ToolError::new(
                    "SEARXNG_INVALID_RESULTS",
                    "SearXNG returned invalid search entries; those entries were omitted.",
                    false,
                ),
            );
        }
        if !page.unresponsive_engines.is_empty() {
            failure(
                result,
                ToolError::new(
                    "SEARXNG_ENGINE_ERROR",
                    format!(
                        "{} upstream search engine(s) failed. Results may be incomplete.",
                        page.unresponsive_engines.len()
                    ),
                    true,
                ),
            );
        }
        if !options.paginate || result.results.len() >= options.limit || !nonempty || result.results.len() == before {
            break;
        }
        page_number += 1;
    }
}

async fn request_page(
    client: &Client,
    endpoint: &Url,
    credentials: &Credentials,
    query: &str,
    page: usize,
    deadline: &Deadline,
) -> Result<Page, ToolError> {
    if deadline.expired() {
        return Err(timeout_error());
    }
    let mut url = endpoint.clone();
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("format", "json")
        .append_pair("pageno", &page.to_string());
    let mut response = client
        .get(url)
        .basic_auth(&credentials.user, Some(&credentials.pass))
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(Duration::from_millis(deadline.remaining_ms().max(1)))
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let (code, message) = match status {
            401 => (
                "SEARXNG_AUTH_FAILED",
                "SearXNG rejected HTTP Basic authentication. Check SEARXNG_USER and SEARXNG_PASS.",
            ),
            403 => (
                "SEARXNG_FORBIDDEN",
                "SearXNG denied this request. Check access permissions and that search.formats includes json in its settings.yml.",
            ),
            429 => ("SEARXNG_RATE_LIMITED", "SearXNG rate-limited the request. Wait before retrying."),
            300..=399 => (
                "SEARXNG_REDIRECT",
                "SearXNG redirected the request. Configure its final base URL; redirects are not followed with credentials.",
            ),
            _ => ("SEARXNG_HTTP_ERROR", "The SearXNG server returned an unsuccessful HTTP status."),
        };
        return Err(ToolError::new(code, message, status == 429 || status >= 500)
            .with_details(serde_json::json!({"status":status})));
    }
    let content_type =
        response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if content_type.split(';').next().unwrap_or("").trim() != "application/json" {
        return Err(invalid_response());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > config::MAX_RESPONSE_BYTES {
            return Err(ToolError::new(
                "SEARXNG_RESPONSE_TOO_LARGE",
                "SearXNG returned more than 8 MiB of JSON.",
                false,
            ));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| invalid_response())
}

fn plain_text(input: &str) -> String {
    let document = Document::from(format!("<html><body>{input}</body></html>").as_str());
    document.select("script,style").remove();
    document.select("body").text().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn invalid_response() -> ToolError {
    ToolError::new(
        "SEARXNG_INVALID_RESPONSE",
        "SearXNG did not return valid search JSON. Check that search.formats includes json in its settings.yml.",
        false,
    )
}

fn timeout_error() -> ToolError {
    ToolError::new("SEARXNG_TIMEOUT", "The SearXNG search exceeded its timeout.", true)
}

fn request_error(error: reqwest::Error) -> ToolError {
    if error.is_timeout() {
        timeout_error()
    } else {
        // Neither raw HTTP errors nor response bodies belong in diagnostics:
        // a proxy or authentication service can echo request credentials.
        ToolError::new(
            "SEARXNG_NETWORK_ERROR",
            "Could not reach SearXNG or read its response. Check connectivity and the TLS certificate.",
            true,
        )
    }
}
