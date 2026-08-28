use crate::types::{FetchResponse, FetchedContent, SearchResponse, ToolError};

pub fn render_search_markdown(response: &SearchResponse) -> String {
    let mut lines = vec![format!("# Search results for `{}`", response.query), String::new()];

    if response.status != crate::types::CommandStatus::Ok {
        let suffix = if response.cached == Some(true) { " (cached)" } else { "" };
        lines.push(format!("_Status: {}{}_", status_word(response.status), suffix));
        lines.push(String::new());
    } else if response.cached == Some(true) {
        lines.push("_Served from cache._".to_string());
        lines.push(String::new());
    }

    for result in &response.results {
        lines.push(format!("{}. [{}]({})", result.rank, result.title, result.url));
        if let Some(display_url) = &result.display_url {
            lines.push(format!("   - URL: {display_url}"));
        }
        if let Some(snippet) = &result.snippet {
            lines.push(format!("   - Snippet: {snippet}"));
        }
        lines.push(String::new());
        if let Some(fetched) = &result.fetched {
            lines.extend(fetched_section(fetched, "###"));
        }
    }

    lines.extend(error_section(&response.errors));
    finish(lines)
}

pub fn render_fetched_markdown(content: &FetchedContent) -> String {
    let heading = content.title.as_deref().unwrap_or(&content.url);
    let mut lines = vec![
        format!("# {heading}"),
        String::new(),
        format!("URL: {}", content.url),
        String::new(),
    ];
    lines.extend(provenance(content));
    if let Some(excerpt) = &content.excerpt {
        lines.push(format!("> {excerpt}"));
        lines.push(String::new());
    }
    if let Some(text) = &content.text {
        lines.push(text.clone());
        lines.push(String::new());
    }
    lines.extend(error_section(&content.errors));
    finish(lines)
}

/// One document per URL, so a failed page is visible instead of silently absent.
pub fn render_fetch_batch_markdown(response: &FetchResponse) -> String {
    let mut lines = vec![
        format!("# Fetched {} URLs", response.results.len()),
        String::new(),
        format!("_Status: {}_", status_word(response.status)),
        String::new(),
    ];

    for content in &response.results {
        let heading = content.title.as_deref().unwrap_or(&content.url);
        lines.push(format!("## {heading}"));
        lines.push(String::new());
        lines.push(format!("URL: {}", content.url));
        lines.push(String::new());
        lines.extend(provenance(content));
        if let Some(excerpt) = &content.excerpt {
            lines.push(format!("> {excerpt}"));
            lines.push(String::new());
        }
        if let Some(text) = &content.text {
            lines.push(text.clone());
            lines.push(String::new());
        }
        for error in &content.errors {
            lines.push(format!("- **{}**: {}", error.code, error.message));
        }
        lines.push(String::new());
    }

    finish(lines)
}

/// Fetched page bodies sit under their own heading rather than indented.
///
/// Three-space indentation reads as a continuation to some parsers and as a code
/// block to others, and the body is the part an agent actually wants intact.
fn fetched_section(content: &FetchedContent, heading: &str) -> Vec<String> {
    let mut lines = vec![format!("{heading} Content of {}", content.url), String::new()];
    lines.extend(provenance(content));
    if let Some(text) = &content.text {
        lines.push(text.clone());
        lines.push(String::new());
    } else if let Some(first) = content.errors.first() {
        lines.push(format!("Could not fetch: {}", first.message));
        lines.push(String::new());
    }
    lines
}

fn provenance(content: &FetchedContent) -> Vec<String> {
    let mut bits: Vec<String> = Vec::new();
    if let Some(byline) = &content.byline {
        bits.push(format!("By {byline}"));
    }
    if let Some(published) = &content.published_time {
        bits.push(published.clone());
    }
    if let Some(method) = content.method {
        bits.push(match content.cached {
            Some(true) => format!("{}, cached", method.as_str()),
            _ => method.as_str().to_string(),
        });
    }
    if content.truncated == Some(true) {
        bits.push(format!("truncated from {} chars", content.text_length()));
    }
    if bits.is_empty() {
        Vec::new()
    } else {
        vec![format!("_{}_", bits.join(" · ")), String::new()]
    }
}

fn error_section(errors: &[ToolError]) -> Vec<String> {
    if errors.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["## Errors".to_string(), String::new()];
    for error in errors {
        lines.push(format!("- **{}**: {}", error.code, error.message));
    }
    lines
}

fn status_word(status: crate::types::CommandStatus) -> &'static str {
    use crate::types::CommandStatus::*;
    match status {
        Ok => "ok",
        Partial => "partial",
        Blocked => "blocked",
        Timeout => "timeout",
        Error => "error",
    }
}

fn finish(lines: Vec<String>) -> String {
    format!("{}\n", lines.join("\n").trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::json::render_json;
    use crate::types::{CommandStatus, SearchEngine, SearchResult};

    fn response() -> SearchResponse {
        SearchResponse {
            query: "example".to_string(),
            engine: SearchEngine::Brave,
            status: CommandStatus::Ok,
            results: vec![SearchResult {
                rank: 1,
                title: "Example".to_string(),
                url: "https://example.com".to_string(),
                display_url: Some("example.com".to_string()),
                snippet: Some("Demo".to_string()),
                fetched: None,
            }],
            cached: None,
            errors: Vec::new(),
        }
    }

    #[test]
    fn renders_pretty_json_that_round_trips() {
        let rendered = render_json(&response());
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed, serde_json::to_value(response()).unwrap());
        assert!(rendered.contains("\n  \"query\""), "expected two-space indentation");
        assert!(rendered.ends_with('\n'));
        // Absent optionals stay absent rather than serialising as null.
        assert!(!rendered.contains("cached"));
        assert!(!rendered.contains("fetched"));
    }

    #[test]
    fn renders_search_markdown() {
        let md = render_search_markdown(&response());
        assert!(md.contains("# Search results for `example`"));
        assert!(md.contains("1. [Example](https://example.com)"));
        assert!(md.contains("Demo"));
    }

    #[test]
    fn renders_fetched_markdown() {
        let mut content = FetchedContent::failure("https://example.com", CommandStatus::Ok, ToolError::new("X", "y", true));
        content.errors.clear();
        content.title = Some("T".to_string());
        content.text = Some("Body".to_string());
        assert!(render_fetched_markdown(&content).contains("# T"));
    }
}
