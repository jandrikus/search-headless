use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandStatus {
    Ok,
    Partial,
    Blocked,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchMethod {
    Http,
    Browser,
}

impl FetchMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            FetchMethod::Http => "http",
            FetchMethod::Browser => "browser",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ToolError {
    pub fn new(code: &str, message: impl Into<String>, recoverable: bool) -> Self {
        Self { code: code.to_string(), message: message.into(), recoverable, details: None }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// A single wall-clock budget threaded through every tier of a command.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    expires_at: Instant,
}

impl Deadline {
    pub fn new(timeout_ms: u64) -> Self {
        Self { expires_at: Instant::now() + Duration::from_millis(timeout_ms) }
    }

    /// Milliseconds left, saturating at zero rather than going negative.
    pub fn remaining_ms(&self) -> u64 {
        self.expires_at.saturating_duration_since(Instant::now()).as_millis() as u64
    }

    pub fn expired(&self) -> bool {
        self.remaining_ms() == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchedContent {
    pub url: String,
    pub status: CommandStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<FetchMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    pub errors: Vec<ToolError>,
}

impl FetchedContent {
    /// A failed fetch: the URL that was asked for, plus why it did not work.
    pub fn failure(url: impl Into<String>, status: CommandStatus, error: ToolError) -> Self {
        Self::failures(url, status, vec![error])
    }

    /// A failed fetch carrying every tier's complaint.
    pub fn failures(url: impl Into<String>, status: CommandStatus, errors: Vec<ToolError>) -> Self {
        Self {
            url: url.into(),
            status,
            title: None,
            site_name: None,
            byline: None,
            published_time: None,
            excerpt: None,
            text: None,
            method: None,
            truncated: None,
            text_length: None,
            cached: None,
            errors,
        }
    }

    pub fn text_length(&self) -> usize {
        self.text_length.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub rank: usize,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched: Option<FetchedContent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    Brave,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub query: String,
    pub engine: SearchEngine,
    pub status: CommandStatus,
    pub results: Vec<SearchResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    pub errors: Vec<ToolError>,
}

/// A `fetch` run over more than one URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchResponse {
    pub status: CommandStatus,
    pub results: Vec<FetchedContent>,
    pub errors: Vec<ToolError>,
}
