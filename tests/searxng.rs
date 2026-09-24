//! End-to-end SearXNG searches through the CLI, using synthetic credentials.
use std::process::{Command, Output};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use search_headless::testing;
use serde_json::{Value, json};

const BINARY: &str = env!("CARGO_BIN_EXE_search-headless");
const TEST_AUTH: &str = "Basic dGVzdC11c2VyOnRlc3QtcGFzcw=="; // test-user:test-pass

struct Sandbox(tempfile::TempDir);
impl Sandbox {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn command(&self, endpoint: &str, query: &str) -> Command {
        self.command_with(endpoint, query, &[])
    }
    fn command_with(&self, endpoint: &str, query: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(BINARY);
        cmd.args(["search", "--engine", "searxng", "--searxng-url", endpoint])
            .args(args)
            .args(["--", query])
            .env("SEARXNG_USER", "test-user")
            .env("SEARXNG_PASS", "test-pass")
            .env("SEARCH_HEADLESS_CACHE_DIR", self.0.path().join("cache"))
            .env("SEARCH_HEADLESS_STATE_DIR", self.0.path().join("state"))
            .env("SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS", "1")
            .env("OBSCURA_BIN", "/nonexistent/obscura")
            .env_remove("SEARCH_HEADLESS_NO_CACHE");
        cmd
    }
    fn run(&self, endpoint: &str) -> Value {
        output_json(self.command(endpoint, "test query").output().unwrap())
    }
}
fn output_json(output: Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}
fn entry(url: &str) -> Value {
    json!({"url":url,"title":"Test &amp; <b>title</b>","content":"A <b>useful</b> snippet."})
}
fn results() -> Value {
    json!({"results":[entry("https://example.org/a"),entry("https://example.org/b")],"unresponsive_engines":[]})
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticates_and_encodes_queries_without_exposing_credentials() {
    let requests = Arc::new(Mutex::new(vec![]));
    let seen = requests.clone();
    let server = testing::serve(move |request| {
        assert_eq!(request.headers().get("authorization").unwrap(), TEST_AUTH);
        seen.lock().unwrap().push(request.uri().to_string());
        testing::with_type(results().to_string(), "application/json")
    })
    .await;
    let sandbox = Sandbox::new();
    let query = "--help + & site:example.org café";
    let output = sandbox.command(&format!("{}/instance/", server.base_url), query).output().unwrap();
    assert!(output.status.success());
    let json = output_json(output);
    assert_eq!(json["engine"], "searxng");
    assert_eq!(json["query"], query);
    assert_eq!(json["results"][0]["title"], "Test & title");
    assert_eq!(json["results"][0]["snippet"], "A useful snippet.");
    assert!(!json.to_string().contains("test-pass"));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let url = url::Url::parse(&format!("http://localhost{}", requests[0])).unwrap();
    assert_eq!(url.path(), "/instance/search");
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(params["q"], query);
    assert_eq!(params["format"], "json");
    assert_eq!(params["pageno"], "1");
}

#[tokio::test(flavor = "multi_thread")]
async fn paginates_only_when_requested_and_deduplicates_results() {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let server = testing::serve(move |request| {
        counter.fetch_add(1, Ordering::SeqCst);
        let page = if request.uri().query().unwrap().contains("pageno=2") {
            json!({"results":[entry("https://example.org/b#top"),entry("https://example.org/c")]})
        } else {
            results()
        };
        testing::with_type(page.to_string(), "application/json")
    })
    .await;
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.run(&server.base_url)["results"].as_array().unwrap().len(), 2);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let result = output_json(
        sandbox.command_with(&server.base_url, "test query", &["--paginate", "--limit", "3"]).output().unwrap(),
    );
    assert_eq!(result["status"], "ok");
    assert_eq!(result["results"].as_array().unwrap().len(), 3);
    assert_eq!(result["results"][2]["rank"], 3);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_upstream_failures_without_discarding_usable_results() {
    let server = testing::serve(|_| {
        testing::with_type(
            json!({
                "results":[entry("https://example.org")], "unresponsive_engines":[["wikidata","timeout"]]
            })
            .to_string(),
            "application/json",
        )
    })
    .await;
    let sandbox = Sandbox::new();
    let json = sandbox.run(&server.base_url);
    assert_eq!(json["status"], "partial");
    assert_eq!(json["results"].as_array().unwrap().len(), 1);
    assert_eq!(json["errors"][0]["code"], "SEARXNG_ENGINE_ERROR");
    assert!(sandbox.run(&server.base_url)["cached"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn distinguishes_empty_results_from_failed_engines() {
    for (failures, status) in [(json!([]), "ok"), (json!([["engine", "timeout"]]), "error")] {
        let server = testing::serve(move |_| {
            testing::with_type(json!({"results":[],"unresponsive_engines":failures}).to_string(), "application/json")
        })
        .await;
        assert_eq!(Sandbox::new().run(&server.base_url)["status"], status);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_credentials_fail_before_any_request() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let server = testing::serve(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        testing::html("unexpected")
    })
    .await;
    let sandbox = Sandbox::new();
    let result = output_json(
        sandbox.command(&server.base_url, "q").env_remove("SEARXNG_USER").env_remove("SEARXNG_PASS").output().unwrap(),
    );
    assert_eq!(result["errors"][0]["code"], "SEARXNG_AUTH_MISSING");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_http_failures_without_echoing_server_bodies() {
    for (status, code) in [
        (401, "SEARXNG_AUTH_FAILED"),
        (403, "SEARXNG_FORBIDDEN"),
        (429, "SEARXNG_RATE_LIMITED"),
        (500, "SEARXNG_HTTP_ERROR"),
    ] {
        let server = testing::serve(move |_| testing::status(status, "test-pass", "text/html")).await;
        let result = Sandbox::new().run(&server.base_url);
        assert_eq!(result["status"], "error");
        assert_eq!(result["errors"][0]["code"], code);
        assert!(!result.to_string().contains("test-pass"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn never_follows_redirects_with_authentication() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let destination = testing::serve(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        testing::html("unexpected")
    })
    .await;
    let url = destination.base_url.clone();
    let source = testing::serve(move |_| testing::redirect(&url)).await;
    let result = Sandbox::new().run(&source.base_url);
    assert_eq!(result["errors"][0]["code"], "SEARXNG_REDIRECT");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_html_and_invalid_json_instead_of_reporting_empty_success() {
    for (body, content_type) in
        [("<html>test-pass</html>", "text/html"), ("not-json", "application/json"), ("{}", "application/json")]
    {
        let server = testing::serve(move |_| testing::with_type(body, content_type)).await;
        let result = Sandbox::new().run(&server.base_url);
        assert_eq!(result["errors"][0]["code"], "SEARXNG_INVALID_RESPONSE");
        assert!(!result.to_string().contains("test-pass"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn caches_successes_separately_for_each_instance() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let server = testing::serve(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        testing::with_type(results().to_string(), "application/json")
    })
    .await;
    let sandbox = Sandbox::new();
    assert!(sandbox.run(&server.base_url)["cached"].is_null());
    assert_eq!(sandbox.run(&server.base_url)["cached"], true);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(sandbox.run(&format!("{}/other-instance", server.base_url))["cached"].is_null());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn result_fetches_do_not_receive_the_searxng_authorization_header() {
    let article = testing::serve(|request| {
        assert!(request.headers().get("authorization").is_none());
        testing::html(format!(
            "<html><head><title>Public article</title></head><body><article><p>{}</p></article></body></html>",
            "Useful article text. ".repeat(100)
        ))
    })
    .await;
    let article_url = format!("{}/article", article.base_url);
    let server = testing::serve(move |request| {
        assert_eq!(request.headers().get("authorization").unwrap(), TEST_AUTH);
        testing::with_type(json!({"results":[entry(&article_url)]}).to_string(), "application/json")
    })
    .await;
    let sandbox = Sandbox::new();
    let result = output_json(sandbox.command_with(&server.base_url, "q", &["--fetch"]).output().unwrap());
    assert_eq!(result["status"], "ok");
    assert_eq!(result["results"][0]["fetched"]["title"], "Public article");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_later_page_failure_preserves_earlier_results() {
    let server = testing::serve(|request| {
        if request.uri().query().unwrap().contains("pageno=2") {
            testing::status(503, "unavailable", "text/plain")
        } else {
            testing::with_type(results().to_string(), "application/json")
        }
    })
    .await;
    let sandbox = Sandbox::new();
    let result = output_json(sandbox.command_with(&server.base_url, "q", &["--paginate"]).output().unwrap());
    assert_eq!(result["status"], "partial");
    assert_eq!(result["results"].as_array().unwrap().len(), 2);
    assert_eq!(result["errors"][0]["code"], "SEARXNG_HTTP_ERROR");
}

#[tokio::test(flavor = "multi_thread")]
async fn skips_unsafe_results_and_marks_the_response_partial() {
    let server = testing::serve(|_| testing::with_type(json!({"results":[
        entry("javascript:alert(1)"), entry("https://test-user:test-pass@example.org"), entry("https://example.org/safe")
    ]}).to_string(), "application/json")).await;
    let result = Sandbox::new().run(&server.base_url);
    assert_eq!(result["status"], "partial");
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["errors"][0]["code"], "SEARXNG_INVALID_RESULTS");
    assert!(!result.to_string().contains("test-pass"));
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_oversized_json_responses() {
    let body = " ".repeat(search_headless::config::MAX_RESPONSE_BYTES + 1);
    let server = testing::serve(move |_| testing::with_type(body.clone(), "application/json")).await;
    let result = Sandbox::new().run(&server.base_url);
    assert_eq!(result["errors"][0]["code"], "SEARXNG_RESPONSE_TOO_LARGE");
}

#[test]
fn rejects_unsafe_urls_without_echoing_credentials() {
    let sandbox = Sandbox::new();
    for url in
        ["https://test-user:test-pass@example.org", "http://example.org", "https://example.org?test-pass", "not-a-url"]
    {
        let result = sandbox.run(url);
        assert_eq!(result["errors"][0]["code"], "SEARXNG_CONFIG_ERROR");
        assert!(!result.to_string().contains("test-pass"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn honors_the_timeout_without_starting_obscura() {
    let server = testing::serve(|_| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        testing::with_type(results().to_string(), "application/json")
    })
    .await;
    let sandbox = Sandbox::new();
    let output = sandbox.command_with(&server.base_url, "q", &["--timeout", "1"]).output().unwrap();
    let result = output_json(output);
    assert_eq!(result["status"], "timeout");
    assert_eq!(result["errors"][0]["code"], "SEARXNG_TIMEOUT");
}
