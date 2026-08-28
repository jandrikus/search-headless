use std::collections::HashSet;

use dom_query::Document;
use url::Url;

use crate::types::SearchResult;

/// Parse Brave Search SERP HTML into structured results.
///
/// Brave uses a Svelte-built DOM with these selectors:
/// - Container: `.snippet[data-type="web"]`, minus `.standalone`
/// - Link: `.result-content a[href]`
/// - Title: `.title` (carries a `title=""` attribute with clean text)
/// - Display URL: `cite.snippet-url`
/// - Description: `.generic-snippet .content`
pub fn parse_brave_results(html: &str, limit: usize, start_rank: usize) -> Vec<SearchResult> {
    let document = Document::from(html);
    let mut out: Vec<SearchResult> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for snippet in document.select(r#".snippet[data-type="web"]"#).iter() {
        if out.len() >= limit {
            break;
        }
        // Standalone snippets are videos, discussions and the like, not organic results.
        if snippet.has_class("standalone") {
            continue;
        }

        let link = snippet.select(".result-content a[href]").first();
        let Some(href) = link.attr("href").map(|h| h.to_string()) else { continue };
        if !href.starts_with("http") || !seen.insert(href.clone()) {
            continue;
        }

        // Prefer the `title` attribute; it is cleaner than the rendered text.
        let title_el = snippet.select(".title").first();
        let title = title_el
            .attr("title")
            .map(|t| t.to_string())
            .unwrap_or_else(|| title_el.text().trim().to_string());
        if title.is_empty() {
            continue;
        }

        out.push(SearchResult {
            rank: start_rank + out.len(),
            title,
            url: href,
            display_url: non_empty(snippet.select("cite.snippet-url").text().trim()),
            snippet: non_empty(snippet.select(".generic-snippet .content").text().trim()),
            fetched: None,
        });
    }

    out
}

/// Parse the "Next" page URL from Brave Search HTML.
///
/// Brave marks it with a link whose text is exactly "Next", carrying `&offset=N`.
pub fn parse_brave_next_url(html: &str) -> Option<String> {
    let document = Document::from(html);
    for anchor in document.select("a").iter() {
        if anchor.text().trim() == "Next" {
            let href = anchor.attr("href")?;
            return Url::parse("https://search.brave.com").ok()?.join(&href).ok().map(|u| u.to_string());
        }
    }
    None
}

/// Phrases Brave uses on its interstitials.
///
/// The CAPTCHA wording is only one of several: a flagged request or a
/// JavaScript-required notice is just as much a block, and matching only the
/// CAPTCHA text reports those pages as an empty success.
const BLOCK_PHRASES: [&str; 7] = [
    "verifying you",
    "verify you are human",
    "not a robot",
    "request has been flagged",
    "unusual traffic",
    "needs javascript",
    "enable javascript",
];

/// Detect whether the page is an interstitial rather than results.
///
/// Only treated as a block when no results were parsed: a page can mention one
/// of these phrases in a snippet without being a challenge.
pub fn detect_brave_block(html: &str, text: &str) -> bool {
    let haystack = text.to_lowercase();
    if !BLOCK_PHRASES.iter().any(|phrase| haystack.contains(phrase)) {
        return false;
    }
    parse_brave_results(html, 1, 1).is_empty()
}

/// Build a Brave Search URL with the given query and options.
pub fn build_brave_search_url(query: &str, offset: Option<usize>) -> String {
    let mut url = Url::parse("https://search.brave.com/search").expect("static URL");
    {
        let mut params = url.query_pairs_mut();
        params.append_pair("q", query);
        if let Some(offset) = offset.filter(|o| *o > 0) {
            params.append_pair("offset", &offset.to_string());
        }
    }
    url.to_string()
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() { None } else { Some(value.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULTS: &str = include_str!("../../tests/fixtures/brave-results.html");
    const BLOCKED: &str = include_str!("../../tests/fixtures/brave-blocked.html");

    #[test]
    fn parses_organic_brave_results() {
        let results = parse_brave_results(RESULTS, 10, 1);

        // Three organic, one standalone skipped.
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].rank, 1);
        assert_eq!(results[0].title, "First Test Article - Example");
        assert_eq!(results[0].url, "https://example.com/article-one");
        assert_eq!(results[0].display_url.as_deref(), Some("example.com › article-one"));
        assert_eq!(
            results[0].snippet.as_deref(),
            Some("This is the first test article snippet with useful information about testing.")
        );
        assert_eq!(results[1].title, "Second Blog Post | Example Org");
        assert_eq!(results[1].url, "https://blog.example.org/second-post");
        assert_eq!(results[2].title, "Documentation Guide");
        assert_eq!(results[2].url, "https://docs.example.com/guide");
    }

    #[test]
    fn skips_standalone_snippets() {
        let urls: Vec<_> = parse_brave_results(RESULTS, 10, 1).into_iter().map(|r| r.url).collect();
        assert!(!urls.iter().any(|u| u == "https://example.com/video"));
    }

    #[test]
    fn respects_the_limit() {
        let results = parse_brave_results(RESULTS, 2, 1);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "First Test Article - Example");
        assert_eq!(results[1].title, "Second Blog Post | Example Org");
    }

    #[test]
    fn parses_the_next_page_url() {
        let next = parse_brave_next_url(RESULTS).expect("a next link");
        assert!(next.contains("search.brave.com/search"));
        assert!(next.contains("offset=1"));
    }

    #[test]
    fn returns_none_when_there_is_no_next_page() {
        assert!(parse_brave_next_url("<html><body><div>No results</div></body></html>").is_none());
    }

    #[test]
    fn detects_a_brave_captcha_block() {
        assert!(detect_brave_block(BLOCKED, "Verifying you're not a bot Quick check before you continue searching."));
    }

    #[test]
    fn does_not_treat_normal_results_as_blocked() {
        assert!(!detect_brave_block("<html><body>Normal results</body></html>", "Normal results"));
    }

    #[test]
    fn detects_interstitials_that_are_not_the_captcha_wording() {
        for text in [
            "Your request has been flagged.",
            "This page needs JavaScript to run.",
            "unusual traffic from your network",
        ] {
            assert!(detect_brave_block("<html><body></body></html>", text), "{text}");
        }
    }

    #[test]
    fn does_not_report_a_block_when_results_parsed_even_if_a_snippet_mentions_a_phrase() {
        assert!(!detect_brave_block(RESULTS, "verifying you can use this library"));
    }

    #[test]
    fn builds_a_search_url_with_and_without_an_offset() {
        assert_eq!(build_brave_search_url("bun typescript", None), "https://search.brave.com/search?q=bun+typescript");
        assert!(build_brave_search_url("x", Some(20)).contains("offset=20"));
    }
}
