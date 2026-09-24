//! Search tests that go through the real obscura spawn path.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use search_headless::cache::CacheOptions;
use search_headless::search::brave::{READ_SERP_SCRIPT, SearchBraveOptions, search_brave};
use search_headless::search::parse_brave::{parse_brave_next_url, parse_brave_results};
use search_headless::types::{CommandStatus, Deadline};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn serp_payload(name: &str) -> String {
    let html = std::fs::read_to_string(fixture(name)).expect("fixture");
    serde_json::json!({ "html": html, "text": html, "url": "https://search.brave.com/search" }).to_string()
}

/// Stand-in obscura that answers with real SERP HTML, so the spawn, parse and
/// pagination path is exercised without a browser. An `offset=` URL gets the
/// second page; a query containing `challenge` gets the blocked fixture.
fn fake_obscura() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (file, source) in [
        ("page1.json", "brave-results.html"),
        ("page2.json", "brave-results-page2.html"),
        ("blocked.json", "brave-blocked.html"),
    ] {
        std::fs::write(dir.path().join(file), serp_payload(source)).expect("write payload");
    }

    let bin = dir.path().join("fake-obscura");
    let here = dir.path().display();
    // argv: fetch <url> --quiet ...; the URL decides which fixture to emit.
    let script = format!(
        "#!/usr/bin/env bash\n\
         url=\"$2\"\n\
         if [[ \"$url\" == *challenge* ]]; then cat {here}/blocked.json\n\
         elif [[ \"$url\" == *offset=* ]]; then cat {here}/page2.json\n\
         else cat {here}/page1.json\n\
         fi\n"
    );
    std::fs::write(&bin, script).expect("write shim");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    (dir, bin)
}

fn options(limit: usize, bin: Option<PathBuf>) -> SearchBraveOptions {
    SearchBraveOptions {
        limit,
        // The cache would answer the second test from the first one's results.
        cache: CacheOptions { enabled: Some(false), ..Default::default() },
        state_dir: bin.as_ref().and_then(|path| path.parent()).map(|parent| parent.join("state")),
        obscura_bin: bin,
        ..SearchBraveOptions::new(Deadline::new(20_000))
    }
}

/// The rate limiter is a shared file; searches in this suite must not queue.
fn without_rate_limiting() {
    unsafe { std::env::set_var("SEARCH_HEADLESS_NO_RATE_LIMIT", "1") };
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_parsed_results_through_the_real_obscura_spawn_path() {
    without_rate_limiting();
    let (_guard, bin) = fake_obscura();

    let response = search_brave("example", &options(2, Some(bin))).await;

    assert_eq!(response.status, CommandStatus::Ok);
    assert_eq!(response.results.len(), 2);
    assert_eq!(response.results[0].title, "First Test Article - Example");
}

#[tokio::test(flavor = "multi_thread")]
async fn paginates_through_the_spawn_path_and_ranks_across_pages() {
    without_rate_limiting();
    let (_guard, bin) = fake_obscura();

    let mut options = options(5, Some(bin));
    options.paginate = true;
    let response = search_brave("example", &options).await;

    assert_eq!(response.results.len(), 5);
    assert_eq!(response.results[4].title, "Fifth Result - Other Site");
    assert_eq!(response.results[4].rank, 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_a_challenge_from_the_spawn_path_as_blocked() {
    without_rate_limiting();
    let (_guard, bin) = fake_obscura();

    let response = search_brave("challenge please", &options(5, Some(bin))).await;

    assert_eq!(response.status, CommandStatus::Blocked);
    assert!(response.results.is_empty());
    assert_eq!(response.errors[0].code, "BRAVE_CHALLENGE");
}

/// A browser that will not start is not a bot challenge. Reporting it as one
/// would send an agent off to wait out a block that does not exist.
#[tokio::test(flavor = "multi_thread")]
async fn reports_a_broken_obscura_as_an_error_not_as_blocked() {
    without_rate_limiting();

    let dir = tempfile::tempdir().unwrap();
    let response = search_brave("anything", &options(3, Some(dir.path().join("missing-obscura")))).await;

    assert_eq!(response.status, CommandStatus::Error);
    assert_eq!(response.errors[0].code, "OBSCURA_SEARCH_FAILED");
    assert!(response.results.is_empty());
}

/// The eval script runs inside obscura, so nothing above would notice if it
/// referenced a property that does not exist. Run it in a real JS engine against
/// a real SERP served locally.
#[tokio::test(flavor = "multi_thread")]
async fn read_serp_script_returns_html_text_and_url_from_a_real_document() {
    let Some(obscura) = search_headless::testing::installed_obscura() else {
        eprintln!("skipping: obscura is not installed");
        return;
    };

    let html = std::fs::read_to_string(fixture("brave-results.html")).expect("fixture");
    let server = search_headless::testing::serve(move |_| search_headless::testing::html(html.clone())).await;
    let url = format!("{}/search?q=test", server.base_url);

    let output = tokio::process::Command::new(obscura)
        .args(["fetch", &url, "--quiet", "--allow-private-network", "--timeout", "30", "--eval", READ_SERP_SCRIPT])
        .output()
        .await
        .expect("run obscura");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "obscura failed: {}", String::from_utf8_lossy(&output.stderr));

    let payload: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON from the eval script");
    assert!(payload["url"].as_str().unwrap().contains("/search"));
    assert!(!payload["text"].as_str().unwrap().is_empty());

    // The captured HTML must still be parseable, including the footer "Next" link.
    let captured = payload["html"].as_str().unwrap();
    assert_eq!(parse_brave_results(captured, 10, 1).len(), 3);
    assert!(parse_brave_next_url(captured).unwrap().contains("offset="));
}
