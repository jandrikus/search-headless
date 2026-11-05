use std::net::SocketAddr;
use std::time::Duration;

use dom_query::Document;
use dom_smoothie::{Config, Readability, TextMode};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, Response, StatusCode};
use url::Url;

use crate::config;
use crate::types::{CommandStatus, Deadline, FetchMethod, FetchedContent, ToolError};

use super::url_validation::{UrlValidationOptions, is_private_ip, validate_public_http_url};

pub struct HttpExtractOptions {
    pub deadline: Deadline,
    pub max_chars: usize,
    pub allow_private_urls: bool,
    pub allow_private_redirects: bool,
    pub max_response_bytes: Option<usize>,
}

impl HttpExtractOptions {
    pub fn new(deadline: Deadline, max_chars: usize) -> Self {
        Self {
            deadline,
            max_chars,
            allow_private_urls: false,
            allow_private_redirects: false,
            max_response_bytes: None,
        }
    }
}

/// Filled in by `extract_via_http` so callers can judge whether the result is
/// trustworthy.
#[derive(Debug, Default)]
pub struct HttpExtractDiagnostics {
    pub script_count: Option<usize>,
    pub content_type: Option<String>,
    pub bytes: Option<usize>,
}

/// Headers a bare HTTP client does not send.
///
/// A default client identifies itself as its own library, and a large share of
/// sites answer an unknown agent with a 403 or a consent wall. Sending what an
/// ordinary browser sends is not evasion - it asks for the same page a person
/// would get, and it is the difference between the cheap tier working and every
/// fetch paying for a browser launch.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

const HTML_TYPES: [&str; 2] = ["text/html", "application/xhtml+xml"];
const MAX_REDIRECTS: usize = 8;

fn client(url: &Url, addresses: &[SocketAddr]) -> Result<Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::USER_AGENT, HeaderValue::from_static(USER_AGENT));
    headers.insert(reqwest::header::ACCEPT, HeaderValue::from_static(ACCEPT));
    headers.insert(reqwest::header::ACCEPT_LANGUAGE, HeaderValue::from_static(ACCEPT_LANGUAGE));
    headers.insert(reqwest::header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));

    let mut builder = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(headers);
    if let Some(url::Host::Domain(host)) = url.host() {
        builder = builder.resolve_to_addrs(host, addresses);
    }
    builder.build().map_err(|_| "FETCH_FAILED".to_string())
}

pub async fn extract_via_http(
    input: &str,
    options: &HttpExtractOptions,
    diagnostics: &mut HttpExtractDiagnostics,
) -> FetchedContent {
    let validation_options = UrlValidationOptions { allow_private_urls: options.allow_private_urls };

    let url = match validate_public_http_url(input, validation_options) {
        Ok(url) => url,
        Err(error) => return FetchedContent::failure(input, CommandStatus::Error, error),
    };
    let response = match fetch_with_redirects(url, options).await {
        Ok(response) => response,
        Err(code) => {
            let status = if code == "FETCH_TIMEOUT" { CommandStatus::Timeout } else { CommandStatus::Error };
            let error = ToolError::new(&code, message_for(&code), code == "FETCH_TIMEOUT");
            return FetchedContent::failure(input, status, error);
        }
    };

    let final_url = response.url().to_string();

    if response.status().as_u16() >= 400 {
        let status = response.status();
        return FetchedContent::failure(
            &final_url,
            CommandStatus::Error,
            ToolError::new(
                "FETCH_HTTP_ERROR",
                format!("Server responded {}.", status_line(status)),
                status.as_u16() >= 500 || status.as_u16() == 429,
            )
            .with_details(serde_json::json!({ "status": status.as_u16() })),
        );
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    diagnostics.content_type = Some(content_type.clone());

    let is_html = content_type.is_empty() || HTML_TYPES.iter().any(|t| content_type.contains(t));
    let is_plain_text = content_type.contains("text/plain") || content_type.contains("text/markdown");
    if !is_html && !is_plain_text {
        let shown = if content_type.is_empty() { "an unknown content type".to_string() } else { content_type.clone() };
        return FetchedContent::failure(
            &final_url,
            CommandStatus::Error,
            ToolError::new("FETCH_NOT_HTML", format!("Expected HTML but got {shown}."), false)
                .with_details(serde_json::json!({ "contentType": content_type })),
        );
    }

    let max_bytes = options.max_response_bytes.unwrap_or(config::MAX_RESPONSE_BYTES);
    let body = match read_bounded(response, max_bytes, &options.deadline).await {
        Ok(body) => body,
        Err(code) => {
            let status = if code == "FETCH_TIMEOUT" { CommandStatus::Timeout } else { CommandStatus::Error };
            return FetchedContent::failure(&final_url, status, ToolError::new(&code, message_for(&code), false));
        }
    };
    diagnostics.bytes = Some(body.len());

    // Plain text needs no article extraction; it is already the article.
    if is_plain_text {
        let text = body.trim();
        if text.is_empty() {
            return FetchedContent::failure(
                &final_url,
                CommandStatus::Error,
                ToolError::new("READABILITY_FAILED", "Response body was empty.", true),
            );
        }
        diagnostics.script_count = Some(0);
        return with_text(Extracted { url: final_url, ..Default::default() }, text, options.max_chars);
    }

    extract_article(&body, final_url, options.max_chars, diagnostics)
}

#[derive(Default)]
struct Extracted {
    url: String,
    title: Option<String>,
    site_name: Option<String>,
    byline: Option<String>,
    published_time: Option<String>,
    excerpt: Option<String>,
}

/// Score the document down to its article, falling back to the whole body.
///
/// The document is parsed twice on purpose: `Readability::parse` mutates its own
/// copy, and both the script count and the body-text fallback have to be read
/// from a document it has not touched.
fn extract_article(
    html: &str,
    final_url: String,
    max_chars: usize,
    diagnostics: &mut HttpExtractDiagnostics,
) -> FetchedContent {
    let document = Document::from(html);
    diagnostics.script_count = Some(document.select("script").length());
    let doc_title = non_empty(document.select("head title").text().to_string());

    let config = Config { text_mode: TextMode::Formatted, ..Default::default() };
    let article = Readability::new(html, Some(final_url.as_str()), Some(config))
        .or_else(|_| Readability::new(html, None, None))
        .ok()
        .and_then(|mut readability| readability.parse().ok());

    let text = article
        .as_ref()
        .map(|a| a.text_content.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| collapse_whitespace(&document.select("body").text()));

    if text.is_empty() {
        return FetchedContent::failure(
            &final_url,
            CommandStatus::Error,
            ToolError::new("READABILITY_FAILED", "Could not extract readable text.", true),
        );
    }

    let extracted = Extracted {
        title: article.as_ref().and_then(|a| non_empty(a.title.clone())).or(doc_title),
        site_name: article.as_ref().and_then(|a| a.site_name.clone()).and_then(non_empty),
        byline: article.as_ref().and_then(|a| a.byline.clone()).and_then(non_empty),
        published_time: article.as_ref().and_then(|a| a.published_time.clone()).and_then(non_empty),
        excerpt: article.as_ref().and_then(|a| a.excerpt.clone()).and_then(non_empty),
        url: final_url,
    };

    with_text(extracted, &text, max_chars)
}

fn with_text(base: Extracted, text: &str, max_chars: usize) -> FetchedContent {
    let text_length = text.chars().count();
    let truncated = text_length > max_chars;
    let body = if truncated { text.chars().take(max_chars).collect() } else { text.to_string() };

    FetchedContent {
        url: base.url,
        status: CommandStatus::Ok,
        title: base.title,
        site_name: base.site_name,
        byline: base.byline,
        published_time: base.published_time,
        excerpt: base.excerpt,
        text: Some(body),
        method: Some(FetchMethod::Http),
        truncated: Some(truncated),
        text_length: Some(text_length),
        cached: None,
        errors: Vec::new(),
    }
}

/// Follow redirects by hand so every hop is re-checked against the SSRF rules.
///
/// A public URL that 302s to 169.254.169.254 is exactly the attack the automatic
/// redirect follower would walk straight into.
async fn fetch_with_redirects(url: Url, options: &HttpExtractOptions) -> Result<Response, String> {
    let mut current = url;

    for hop in 0..MAX_REDIRECTS {
        let remaining = options.deadline.remaining_ms();
        if remaining == 0 {
            return Err("FETCH_TIMEOUT".to_string());
        }

        let host = match current.host() {
            Some(url::Host::Domain(host)) => host.to_string(),
            Some(url::Host::Ipv4(ip)) => ip.to_string(),
            Some(url::Host::Ipv6(ip)) => ip.to_string(),
            None => return Err("INVALID_URL".to_string()),
        };
        let port = current.port_or_known_default().ok_or("INVALID_URL")?;
        let addresses: Vec<_> = match tokio::time::timeout(
            Duration::from_millis(remaining),
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        {
            Ok(Ok(addresses)) => addresses.collect(),
            Ok(Err(_)) => return Err("FETCH_FAILED".to_string()),
            Err(_) => return Err("FETCH_TIMEOUT".to_string()),
        };
        let allow_private = if hop == 0 { options.allow_private_urls } else {
            options.allow_private_urls && options.allow_private_redirects
        };
        if addresses.is_empty() {
            return Err("FETCH_FAILED".to_string());
        }
        if !allow_private && addresses.iter().any(|address| is_private_ip(address.ip())) {
            return Err("PRIVATE_URL_BLOCKED".to_string());
        }
        let client = client(&current, &addresses)?;
        let remaining = options.deadline.remaining_ms();
        if remaining == 0 {
            return Err("FETCH_TIMEOUT".to_string());
        }
        let request = client.get(current.clone()).send();
        let response = match tokio::time::timeout(Duration::from_millis(remaining), request).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) if e.is_timeout() => return Err("FETCH_TIMEOUT".to_string()),
            Ok(Err(_)) => return Err("FETCH_FAILED".to_string()),
            Err(_) => return Err("FETCH_TIMEOUT".to_string()),
        };

        if !is_redirect(response.status()) {
            return Ok(response);
        }
        let Some(location) = response.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) else {
            return Ok(response);
        };
        let Ok(next) = current.join(location) else {
            return Ok(response);
        };

        // A redirect is only allowed to reach a private address when private
        // redirects were asked for explicitly, not merely private URLs.
        let hop_options = UrlValidationOptions {
            allow_private_urls: options.allow_private_redirects && options.allow_private_urls,
        };
        current = validate_public_http_url(next.as_str(), hop_options).map_err(|e| e.code)?;
    }

    Err("TOO_MANY_REDIRECTS".to_string())
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

/// Read the body, stopping once it exceeds the cap.
///
/// Buffering the whole thing and checking afterwards is too late on a
/// multi-gigabyte file, so the cap has to be enforced while reading.
async fn read_bounded(mut response: Response, max_bytes: usize, deadline: &Deadline) -> Result<String, String> {
    if let Some(declared) = response.content_length()
        && declared > max_bytes as u64
    {
        return Err("RESPONSE_TOO_LARGE".to_string());
    }

    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let remaining = deadline.remaining_ms();
        if remaining == 0 {
            return Err("FETCH_TIMEOUT".to_string());
        }
        let chunk = match tokio::time::timeout(Duration::from_millis(remaining), response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => return Err("FETCH_FAILED".to_string()),
            Err(_) => return Err("FETCH_TIMEOUT".to_string()),
        };
        if buffer.len() + chunk.len() > max_bytes {
            return Err("RESPONSE_TOO_LARGE".to_string());
        }
        buffer.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn message_for(code: &str) -> String {
    match code {
        "PRIVATE_URL_BLOCKED" => "URL resolves to a private or local address.".to_string(),
        "RESPONSE_TOO_LARGE" => "Response exceeded the size limit.".to_string(),
        "TOO_MANY_REDIRECTS" => "Too many redirects.".to_string(),
        "FETCH_TIMEOUT" => "The request did not finish before the deadline.".to_string(),
        other => other.to_string(),
    }
}

fn status_line(status: StatusCode) -> String {
    match status.canonical_reason() {
        Some(reason) => format!("{} {}", status.as_u16(), reason),
        None => status.as_u16().to_string(),
    }
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, TestServer};
    use std::sync::Arc;

    const ARTICLE: &str = include_str!("../../tests/fixtures/article.html");

    fn options(timeout_ms: u64, max_chars: usize) -> HttpExtractOptions {
        HttpExtractOptions { allow_private_urls: true, ..HttpExtractOptions::new(Deadline::new(timeout_ms), max_chars) }
    }

    async fn article_server() -> TestServer {
        testing::serve(|_| testing::html(ARTICLE)).await
    }

    async fn run(url: &str, options: &HttpExtractOptions) -> FetchedContent {
        extract_via_http(url, options, &mut HttpExtractDiagnostics::default()).await
    }

    #[tokio::test]
    async fn connects_to_the_validated_address_without_resolving_the_name_again() {
        let server = testing::serve(|_| testing::with_type("validated address", "text/plain")).await;
        let local = Url::parse(&server.base_url).unwrap();
        let address: SocketAddr = format!("127.0.0.1:{}", local.port().unwrap()).parse().unwrap();
        // This name cannot resolve in DNS. The client must use the supplied address.
        let url = Url::parse(&format!("http://rebind.invalid:{}/", local.port().unwrap())).unwrap();
        let response = client(&url, &[address]).unwrap().get(url).send().await.unwrap();
        assert_eq!(response.text().await.unwrap(), "validated address");
    }

    #[tokio::test]
    async fn extracts_readable_content_and_metadata_over_http() {
        let server = article_server().await;
        let result = run(&format!("{}/article", server.base_url), &options(5000, 50_000)).await;

        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.method, Some(FetchMethod::Http));
        assert_eq!(result.title.as_deref(), Some("Fixture Article"));
        assert_eq!(result.site_name.as_deref(), Some("Fixture Site"));
        assert_eq!(result.byline.as_deref(), Some("Test Author"));
        assert_eq!(result.published_time.as_deref(), Some("2026-04-26"));
        assert_eq!(result.excerpt.as_deref(), Some("Fixture excerpt"));
        assert!(result.text.unwrap().contains("useful paragraph"));
    }

    #[tokio::test]
    async fn truncates_text_and_preserves_original_length() {
        let server = article_server().await;
        let result = run(&format!("{}/article", server.base_url), &options(5000, 20)).await;

        assert_eq!(result.truncated, Some(true));
        assert!(result.text.as_ref().unwrap().chars().count() <= 20);
        assert!(result.text_length() > 20);
    }

    #[tokio::test]
    async fn blocks_redirects_to_private_urls_by_default() {
        let server = testing::serve(|_| testing::redirect("http://127.0.0.1/private")).await;
        let mut options = options(5000, 1000);
        options.allow_private_redirects = false;

        let result = run(&format!("{}/redirect", server.base_url), &options).await;
        assert_eq!(result.status, CommandStatus::Error);
        assert_eq!(result.errors[0].code, "PRIVATE_URL_BLOCKED");
    }

    #[tokio::test]
    async fn identifies_as_a_browser_so_ordinary_sites_answer() {
        // A default library agent string is answered with a 403 by a large share
        // of sites, which would push every one of them onto the expensive tier.
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let recorder = seen.clone();
        let server = testing::serve(move |request| {
            let agent = request
                .headers()
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            *recorder.lock().unwrap() = agent;
            testing::html("<html><body><p>ok</p></body></html>")
        })
        .await;

        run(&format!("{}/a", server.base_url), &options(5000, 1000)).await;
        let agent = seen.lock().unwrap().clone();
        assert!(agent.contains("Mozilla/5.0"), "got {agent}");
        assert!(!agent.contains("reqwest"), "got {agent}");
    }

    #[tokio::test]
    async fn reports_an_http_error_status_instead_of_parsing_the_error_page() {
        let server = testing::serve(|_| testing::status(404, "<html><body>Not found</body></html>", "text/html")).await;
        let result = run(&format!("{}/missing", server.base_url), &options(5000, 1000)).await;

        assert_eq!(result.status, CommandStatus::Error);
        assert_eq!(result.errors[0].code, "FETCH_HTTP_ERROR");
        assert_eq!(result.errors[0].details, Some(serde_json::json!({ "status": 404 })));
    }

    #[tokio::test]
    async fn returns_plain_text_bodies_as_is() {
        let server = testing::serve(|_| testing::with_type("# Notes\n\nJust text.", "text/plain; charset=utf-8")).await;
        let result = run(&format!("{}/notes.txt", server.base_url), &options(5000, 1000)).await;

        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.text.unwrap().contains("Just text."));
    }

    #[tokio::test]
    async fn refuses_a_body_larger_than_the_cap_instead_of_buffering_it() {
        let body = format!("<html><body>{}</body></html>", "x".repeat(200_000));
        let server = testing::serve(move |_| testing::html(body.clone())).await;
        let mut options = options(5000, 1000);
        options.max_response_bytes = Some(1024);

        let result = run(&format!("{}/big", server.base_url), &options).await;
        assert_eq!(result.status, CommandStatus::Error);
        assert_eq!(result.errors[0].code, "RESPONSE_TOO_LARGE");
    }

    #[tokio::test]
    async fn rejects_a_non_html_content_type_without_downloading_it() {
        let server = testing::serve(|_| testing::with_type("%PDF-1.7", "application/pdf")).await;
        let result = run(&format!("{}/doc.pdf", server.base_url), &options(5000, 1000)).await;

        assert_eq!(result.status, CommandStatus::Error);
        assert_eq!(result.errors[0].code, "FETCH_NOT_HTML");
        assert!(result.errors[0].message.contains("application/pdf"));
    }

    #[tokio::test]
    async fn records_the_script_count_so_a_thin_page_can_be_judged() {
        let server = testing::serve(|_| {
            testing::html("<html><body><script>1</script><script>2</script><p>hi</p></body></html>")
        })
        .await;
        let mut diagnostics = HttpExtractDiagnostics::default();
        extract_via_http(&format!("{}/a", server.base_url), &options(5000, 1000), &mut diagnostics).await;

        assert_eq!(diagnostics.script_count, Some(2));
    }
}
