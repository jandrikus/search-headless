//! Domain-specific URL rewrites for sites that block headless browsers.
//!
//! These rewrites are applied before any extraction attempt, so an anti-bot
//! JavaScript challenge is sidestepped by asking an alternative endpoint that
//! serves the same content.

use url::Url;

/// Apply domain-specific URL rewrites.
/// Returns the rewritten URL, or the input unchanged if no rule matched.
pub fn rewrite_url(input: &str) -> String {
    let Ok(mut url) = Url::parse(input) else {
        return input.to_string();
    };

    // Reddit: www.reddit.com serves JS anti-bot challenges to headless browsers.
    // old.reddit.com serves the same content without challenges.
    if url.host_str() == Some("www.reddit.com") && url.set_host(Some("old.reddit.com")).is_ok() {
        return url.to_string();
    }

    input.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_www_reddit_to_old_reddit() {
        assert_eq!(
            rewrite_url("https://www.reddit.com/r/rust/comments/abc/title/"),
            "https://old.reddit.com/r/rust/comments/abc/title/"
        );
    }

    #[test]
    fn leaves_other_urls_alone() {
        assert_eq!(rewrite_url("https://example.com/a"), "https://example.com/a");
        assert_eq!(rewrite_url("https://old.reddit.com/r/rust"), "https://old.reddit.com/r/rust");
    }

    #[test]
    fn returns_an_unparseable_input_unchanged() {
        assert_eq!(rewrite_url("not a url"), "not a url");
    }
}
