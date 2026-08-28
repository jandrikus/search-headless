//! The browser tier, driven by the real obscura binary.
//!
//! `EXTRACT_SCRIPT` runs inside obscura's JavaScript engine, so nothing in the
//! Rust type system would notice if it referenced a property that does not
//! exist. Escalation to the browser is covered by unit tests with a stand-in
//! binary; this covers the script itself, against a page that genuinely needs
//! JavaScript to have any content at all.

use search_headless::fetch::{FetchContentOptions, fetch_content};
use search_headless::testing::{self, installed_obscura};
use search_headless::types::{CommandStatus, Deadline, FetchMethod};

const SPA: &str = r#"<!doctype html><html><head><title>SPA Shell</title></head><body>
<nav>Home About</nav><div id="root"><p>Loading…</p></div>
<script>
document.getElementById('root').innerHTML = '<article><p>' + 'This paragraph only exists after JavaScript ran, and it is long enough to clear the content threshold. '.repeat(8) + '</p></article>';
</script></body></html>"#;

#[tokio::test(flavor = "multi_thread")]
async fn renders_a_javascript_only_page_through_the_real_browser_tier() {
    if installed_obscura().is_none() {
        eprintln!("skipping: obscura is not installed");
        return;
    }

    let server = testing::serve(|_| testing::html(SPA)).await;
    let options = FetchContentOptions {
        max_chars: 50_000,
        allow_private_urls: true,
        cache: search_headless::cache::CacheOptions { enabled: Some(false), ..Default::default() },
        ..FetchContentOptions::new(Deadline::new(60_000))
    };

    let result = fetch_content(&format!("{}/index.html", server.base_url), &options).await;

    assert_eq!(result.status, CommandStatus::Ok, "{:?}", result.errors);
    assert_eq!(result.method, Some(FetchMethod::Browser));
    assert_eq!(result.title.as_deref(), Some("SPA Shell"));
    let text = result.text.unwrap();
    assert!(text.contains("only exists after JavaScript ran"), "{text}");
    // `nav` is stripped by EXTRACT_SCRIPT before the text is read.
    assert!(!text.contains("Home About"), "{text}");
}
