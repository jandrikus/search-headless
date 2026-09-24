use std::collections::HashSet;

use dom_query::Document;
use url::Url;

use crate::types::{SearchResult, ToolError};

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
        let title = title_el.attr("title").map(|t| t.to_string()).unwrap_or_else(|| title_el.text().trim().to_string());
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
            let url = Url::parse("https://search.brave.com").ok()?.join(&href).ok()?;
            if url.scheme() == "https"
                && url.host_str() == Some("search.brave.com")
                && url.path() == "/search"
                && url.port_or_known_default() == Some(443)
                && url.username().is_empty()
                && url.password().is_none()
            {
                return Some(url.to_string());
            }
        }
    }
    None
}

const BLOCK_PHRASES: [&str; 5] =
    ["verifying you", "verify you are human", "not a robot", "request has been flagged", "unusual traffic"];

/// Classify empty pages instead of treating a parser or rendering failure as
/// zero matches. Diagnostics contain counts and a URL without query parameters,
/// never cookies, storage contents, script bodies, or a dump of the page.
pub fn diagnose_brave_page(html: &str, final_url: &str) -> Option<ToolError> {
    if !parse_brave_results(html, 1, 1).is_empty() {
        return None;
    }
    let document = Document::from(html);
    let snippets = document.select(r#".snippet[data-type="web"]:not(.standalone)"#).length();
    let links = document.select(".result-content a[href]").length();
    document.select("script,style,template,svg").remove();
    // A result whose markup changed must not turn a quoted challenge message
    // into an actual challenge. Keep interstitial text outside result cards.
    document.select(".snippet, .result-content").remove();
    let javascript_notice = document.select("noscript").text().to_lowercase();
    document.select("noscript").remove();
    let text = document.select("body").text().to_lowercase();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let challenge = BLOCK_PHRASES.iter().any(|phrase| text.contains(phrase));
    let javascript = ["needs javascript", "enable javascript", "javascript is required"]
        .iter()
        .any(|phrase| text.contains(phrase) || javascript_notice.contains(phrase));
    let no_results = document.select("h1,h2,h3,[role=heading]").iter().any(|heading| {
        let text = heading.text().to_lowercase();
        let text = text.trim().trim_end_matches(['.', '!']);
        matches!(text, "no results" | "no results found" | "no matching results" | "no matches found")
    });

    let (code, message) = if challenge {
        ("BRAVE_CHALLENGE", "Brave Search served a bot challenge instead of results.")
    } else if snippets > 0 || links > 0 {
        (
            "BRAVE_PARSE_ERROR",
            "Brave returned result markup, but no results could be parsed. Its selectors may have changed.",
        )
    } else if no_results {
        return None;
    } else if javascript {
        (
            "BRAVE_JAVASCRIPT_REQUIRED",
            "Brave returned a JavaScript-required notice. Obscura did not render the results page.",
        )
    } else {
        (
            "BRAVE_UNRECOGNIZED_PAGE",
            "Brave returned an unrecognized page, not a confirmed empty result set. Rendering may be incomplete or the page markup may have changed.",
        )
    };

    let page_url = Url::parse(final_url).ok().map(|mut url| {
        url.set_query(None);
        url.set_fragment(None);
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.to_string()
    });
    Some(ToolError::new(code, message, code == "BRAVE_CHALLENGE").with_details(serde_json::json!({
        "pageUrl": page_url,
        "title": document.select("title").text().chars().take(160).collect::<String>(),
        "webSnippetCount": snippets,
        "resultLinkCount": links,
    })))
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
        assert_eq!(diagnose_brave_page(BLOCKED, "").unwrap().code, "BRAVE_CHALLENGE");
    }

    #[test]
    fn does_not_treat_normal_results_as_blocked() {
        assert_eq!(
            diagnose_brave_page("<html><body>Normal results</body></html>", "").unwrap().code,
            "BRAVE_UNRECOGNIZED_PAGE"
        );
    }

    #[test]
    fn detects_interstitials_that_are_not_the_captcha_wording() {
        for text in ["Your request has been flagged.", "unusual traffic from your network"] {
            assert_eq!(
                diagnose_brave_page(&format!("<html><body>{text}</body></html>"), "").unwrap().code,
                "BRAVE_CHALLENGE"
            );
        }
    }

    #[test]
    fn does_not_report_a_block_when_results_parsed_even_if_a_snippet_mentions_a_phrase() {
        let html = RESULTS.replace("useful information", "unusual traffic");
        assert!(diagnose_brave_page(&html, "").is_none());
    }

    #[test]
    fn ignores_challenge_strings_in_scripts_and_unparseable_result_cards() {
        let html = r#"<html><body><script>const label = 'verify you are human';</script>
            <div class="snippet" data-type="web">unusual traffic</div></body></html>"#;
        assert_eq!(diagnose_brave_page(html, "").unwrap().code, "BRAVE_PARSE_ERROR");
    }

    #[test]
    fn explicit_no_matches_outranks_an_inert_noscript_notice() {
        let html = "<html><body><noscript>Please enable JavaScript</noscript><h1>No results found</h1></body></html>";
        assert!(diagnose_brave_page(html, "").is_none());
    }

    #[test]
    fn javascript_notices_are_not_bot_challenges() {
        let html = "<html><body><noscript>This page needs JavaScript</noscript></body></html>";
        assert_eq!(diagnose_brave_page(html, "").unwrap().code, "BRAVE_JAVASCRIPT_REQUIRED");
    }

    #[test]
    fn diagnostics_do_not_include_query_parameters_or_page_contents() {
        let html = "<html><head><title>Loading</title></head><body>private page body</body></html>";
        let error = diagnose_brave_page(html, "https://search.brave.com/search?q=private-query#fragment").unwrap();
        let details = error.details.unwrap();
        assert_eq!(details["pageUrl"], "https://search.brave.com/search");
        assert!(!details.to_string().contains("private"));
    }

    #[test]
    fn pagination_cannot_navigate_away_from_brave_search() {
        for href in [
            "https://example.com/search?offset=1",
            "http://search.brave.com/search?offset=1",
            "/other",
            "https://search.brave.com:8443/search",
        ] {
            assert!(parse_brave_next_url(&format!(r#"<a href="{href}">Next</a>"#)).is_none());
        }
    }

    #[test]
    fn builds_a_search_url_with_and_without_an_offset() {
        assert_eq!(build_brave_search_url("bun typescript", None), "https://search.brave.com/search?q=bun+typescript");
        assert!(build_brave_search_url("x", Some(20)).contains("offset=20"));
    }
}
