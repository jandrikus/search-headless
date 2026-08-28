use std::path::PathBuf;

use serde::Deserialize;

use crate::obscura::{ObscuraFetchArgs, obscura_fetch, parse_obscura_json};
use crate::types::{CommandStatus, Deadline, FetchMethod, FetchedContent, ToolError};

use super::url_validation::{UrlValidationOptions, assert_public_dns, validate_public_http_url};

pub struct ObscuraExtractOptions {
    pub deadline: Deadline,
    pub max_chars: usize,
    pub allow_private_urls: bool,
    /// Override the obscura binary. `None` reads `OBSCURA_BIN`, then PATH.
    pub obscura_bin: Option<PathBuf>,
}

/// Extract title, final URL and body text.
///
/// `noscript` is stripped alongside script/style: obscura has no layout engine,
/// so `innerText` is really `textContent` and would otherwise include `<noscript>`
/// blocks that a real browser never renders.
pub const EXTRACT_SCRIPT: &str = concat!(
    "(() => {",
    "  document.querySelectorAll('script,style,noscript,nav,footer,aside').forEach(el => el.remove());",
    "  return JSON.stringify({title: document.title, url: window.location.href, text: document.body ? document.body.innerText : ''});",
    "})()"
);

#[derive(Debug, Default, Deserialize)]
struct Extracted {
    title: Option<String>,
    url: Option<String>,
    text: Option<String>,
}

pub async fn extract_via_obscura(input: &str, options: &ObscuraExtractOptions) -> FetchedContent {
    if options.deadline.remaining_ms() <= 1000 {
        return FetchedContent::failure(
            input,
            CommandStatus::Timeout,
            ToolError::new("FETCH_TIMEOUT", "Overall timeout expired before obscura extraction could start.", true),
        );
    }

    let validation_options = UrlValidationOptions { allow_private_urls: options.allow_private_urls };
    let url = match validate_public_http_url(input, validation_options) {
        Ok(url) => url,
        Err(error) => return FetchedContent::failure(input, CommandStatus::Error, error),
    };
    if let Some(error) = assert_public_dns(&url, validation_options).await {
        return FetchedContent::failure(input, CommandStatus::Error, error);
    }

    let mut args = ObscuraFetchArgs::new(input, options.deadline)
        .eval(EXTRACT_SCRIPT)
        .stealth(true)
        .allow_private_network(options.allow_private_urls);
    args.bin = options.obscura_bin.clone();

    let result = obscura_fetch(&args).await;

    if !result.ok {
        let timed_out = options.deadline.expired();
        let message = if result.stderr.trim().is_empty() {
            format!("obscura exited with code {}.", result.exit_code)
        } else {
            result.stderr.trim().to_string()
        };
        return FetchedContent::failure(
            input,
            if timed_out { CommandStatus::Timeout } else { CommandStatus::Error },
            ToolError::new(
                if timed_out { "FETCH_TIMEOUT" } else { "BROWSER_NAVIGATION_FAILED" },
                message,
                true,
            ),
        );
    }

    let Some(parsed) = parse_obscura_json::<Extracted>(&result.stdout) else {
        return FetchedContent::failure(
            input,
            CommandStatus::Error,
            ToolError::new("READABILITY_FAILED", "obscura returned no usable extraction result.", true),
        );
    };

    let text = parsed.text.unwrap_or_default().trim().to_string();
    let text_length = text.chars().count();
    let truncated = text_length > options.max_chars;
    let body: String = if truncated { text.chars().take(options.max_chars).collect() } else { text.clone() };

    FetchedContent {
        url: parsed.url.filter(|u| !u.is_empty()).unwrap_or_else(|| input.to_string()),
        status: if text.is_empty() { CommandStatus::Error } else { CommandStatus::Ok },
        title: parsed.title.filter(|t| !t.is_empty()),
        site_name: None,
        byline: None,
        published_time: None,
        excerpt: None,
        text: Some(body),
        method: Some(FetchMethod::Browser),
        truncated: Some(truncated),
        text_length: Some(text_length),
        cached: None,
        errors: if text.is_empty() {
            vec![ToolError::new("READABILITY_FAILED", "Could not extract rendered text.", true)]
        } else {
            Vec::new()
        },
    }
}
