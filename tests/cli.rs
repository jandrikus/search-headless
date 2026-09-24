//! End-to-end tests that run the real binary, the way an agent calls it.

use std::path::Path;
use std::process::Output;

use search_headless::testing::{self, TestServer};

const BINARY: &str = env!("CARGO_BIN_EXE_search-headless");

struct Run {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout was not JSON ({e}):\n{}\nstderr:\n{}", self.stdout, self.stderr))
    }
}

/// A sandbox that keeps the suite off the developer's real cache and rate-limit
/// files, and off the network's rate limit.
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().expect("tempdir") }
    }

    fn run(&self, command: &str, args: &[&str]) -> Run {
        self.run_with(command, args, &[])
    }

    fn run_with(&self, command: &str, args: &[&str], env: &[(&str, &str)]) -> Run {
        let mut process = std::process::Command::new(BINARY);
        process
            .arg(command)
            .args(args)
            .env("SEARCH_HEADLESS_CACHE_DIR", self.dir.path().join("cache"))
            .env("SEARCH_HEADLESS_RATE_FILE", self.dir.path().join("last-search"))
            .env("SEARCH_HEADLESS_STATE_DIR", self.dir.path().join("state"))
            .env("SEARCH_HEADLESS_NO_RATE_LIMIT", "1")
            // No fake obscura unless a test installs one, so an unexpected
            // escalation to the browser fails loudly instead of hanging.
            .env("OBSCURA_BIN", "/nonexistent/obscura");
        for (key, value) in env {
            process.env(key, value);
        }
        finish(process.output().expect("run the binary"))
    }
}

fn finish(output: Output) -> Run {
    Run {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name).to_string_lossy().into_owned()
}

async fn serve_titled_pages() -> TestServer {
    testing::serve(|request| {
        let name = request.uri().path().trim_start_matches('/').to_string();
        let filler = "filler ".repeat(80);
        testing::html(format!(
            "<!doctype html><html><head><title>{name}</title></head><body><article><p>Body of {name}. {filler}</p></article></body></html>"
        ))
    })
    .await
}

#[test]
fn fetch_rejects_private_urls_by_default_with_json() {
    let sandbox = Sandbox::new();
    let run = sandbox.run("fetch", &["http://127.0.0.1:1"]);

    assert_ne!(run.exit_code, 0);
    assert_eq!(run.json()["status"], "error");
    assert_eq!(run.json()["errors"][0]["code"], "PRIVATE_URL_BLOCKED");
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_extracts_a_local_fixture_only_in_test_mode() {
    let article = std::fs::read_to_string(fixture("article.html")).unwrap();
    let server = testing::serve(move |_| testing::html(article.clone())).await;
    let sandbox = Sandbox::new();

    let run = sandbox.run_with(
        "fetch",
        &[&format!("{}/a", server.base_url)],
        &[("SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS", "1")],
    );

    assert_eq!(run.exit_code, 0, "{}", run.stderr);
    assert_eq!(run.json()["status"], "ok");
    assert!(run.json()["text"].as_str().unwrap().contains("useful paragraph"));
}

#[test]
fn search_fixture_mode_returns_parsed_json() {
    let sandbox = Sandbox::new();
    let run = sandbox.run_with(
        "search",
        &["example", "--limit", "2"],
        &[("SEARCH_HEADLESS_BRAVE_FIXTURE", &fixture("brave-results.html"))],
    );

    assert_eq!(run.exit_code, 0, "{}", run.stderr);
    let json = run.json();
    assert_eq!(json["engine"], "brave");
    assert_eq!(json["results"].as_array().unwrap().len(), 2);
    assert_eq!(json["results"][0]["title"], "First Test Article - Example");
}

#[test]
fn search_fixture_mode_paginates_only_when_explicitly_requested() {
    let sandbox = Sandbox::new();
    let run = sandbox.run_with(
        "search",
        &["example", "--limit", "5", "--paginate"],
        &[
            ("SEARCH_HEADLESS_BRAVE_FIXTURE", &fixture("brave-results.html")),
            ("SEARCH_HEADLESS_BRAVE_FIXTURE_2", &fixture("brave-results-page2.html")),
        ],
    );

    assert_eq!(run.exit_code, 0, "{}", run.stderr);
    let json = run.json();
    assert_eq!(json["results"].as_array().unwrap().len(), 5);
    assert_eq!(json["results"][4]["title"], "Fifth Result - Other Site");
    assert_eq!(json["results"][4]["rank"], 5);
}

#[test]
fn search_reports_a_challenge_as_blocked_never_as_an_empty_success() {
    let sandbox = Sandbox::new();
    let run =
        sandbox.run_with("search", &["example"], &[("SEARCH_HEADLESS_BRAVE_FIXTURE", &fixture("brave-blocked.html"))]);

    assert_ne!(run.exit_code, 0);
    let json = run.json();
    assert_eq!(json["status"], "blocked");
    assert_eq!(json["results"].as_array().unwrap().len(), 0);
    assert_eq!(json["errors"][0]["code"], "BRAVE_CHALLENGE");
}

#[test]
fn search_keeps_a_later_page_challenge_terminal_exiting_non_zero_with_earlier_results() {
    let sandbox = Sandbox::new();
    let run = sandbox.run_with(
        "search",
        &["example", "--limit", "5", "--paginate"],
        &[
            ("SEARCH_HEADLESS_BRAVE_FIXTURE", &fixture("brave-results.html")),
            ("SEARCH_HEADLESS_BRAVE_FIXTURE_2", &fixture("brave-blocked.html")),
        ],
    );

    assert_ne!(run.exit_code, 0);
    let json = run.json();
    assert_eq!(json["status"], "blocked");
    assert_eq!(json["results"].as_array().unwrap().len(), 3);
    assert_eq!(json["errors"][0]["code"], "BRAVE_CHALLENGE");
}

#[test]
fn search_rejects_the_removed_google_flag() {
    let sandbox = Sandbox::new();
    let run = sandbox.run("search", &["example", "--google"]);

    assert_ne!(run.exit_code, 0);
    assert!(run.stderr.contains("--google"), "{}", run.stderr);
}

#[test]
fn fetch_with_an_exhausted_timeout_returns_a_structured_timeout() {
    // With a 1ms timeout every tier declines to start.
    let sandbox = Sandbox::new();
    let run = sandbox.run("fetch", &["https://example.com", "--timeout", "1"]);

    assert_ne!(run.exit_code, 0);
    assert!(!run.stdout.is_empty());
    let json = run.json();
    assert!(json["status"] == "timeout" || json["status"] == "error", "got {}", json["status"]);
    assert_eq!(json["errors"][0]["code"], "FETCH_TIMEOUT");
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_takes_several_urls_in_one_run_and_keeps_them_in_order() {
    let server = serve_titled_pages().await;
    let sandbox = Sandbox::new();

    let run = sandbox.run_with(
        "fetch",
        &[&format!("{}/alpha", server.base_url), &format!("{}/beta", server.base_url)],
        &[("SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS", "1")],
    );

    assert_eq!(run.exit_code, 0, "{}", run.stderr);
    let json = run.json();
    assert_eq!(json["status"], "ok");
    let titles: Vec<_> = json["results"].as_array().unwrap().iter().map(|r| r["title"].clone()).collect();
    assert_eq!(titles, vec!["alpha", "beta"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_reports_a_batch_where_only_some_urls_worked_as_partial_exiting_zero() {
    let server = testing::serve(|request| {
        if request.uri().path() == "/good" {
            let body = "real content ".repeat(60);
            testing::html(format!(
                "<!doctype html><html><head><title>Good</title></head><body><article><p>{body}</p></article></body></html>"
            ))
        } else {
            testing::status(404, "nope", "text/html")
        }
    })
    .await;
    let sandbox = Sandbox::new();

    let run = sandbox.run_with(
        "fetch",
        &[&format!("{}/good", server.base_url), &format!("{}/bad", server.base_url)],
        &[("SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS", "1")],
    );

    assert_eq!(run.exit_code, 0, "{}", run.stderr);
    let json = run.json();
    assert_eq!(json["status"], "partial");
    assert_eq!(json["results"][0]["status"], "ok");
    assert_eq!(json["results"][1]["status"], "error");
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_no_cache_re_requests_the_page() {
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    let server = testing::serve(move |_| {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = "content ".repeat(80);
        testing::html(format!(
            "<!doctype html><html><head><title>Counted</title></head><body><article><p>{body}</p></article></body></html>"
        ))
    })
    .await;

    let sandbox = Sandbox::new();
    let url = format!("{}/page", server.base_url);
    let env = [("SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS", "1")];

    sandbox.run_with("fetch", &[&url], &env);
    sandbox.run_with("fetch", &[&url], &env);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);

    sandbox.run_with("fetch", &[&url, "--no-cache"], &env);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn rejects_a_nonsense_flag_value_before_doing_any_work() {
    let sandbox = Sandbox::new();
    let run = sandbox.run("search", &["example", "--limit", "0"]);

    assert_eq!(run.exit_code, 2);
    assert!(run.stderr.contains("positive integer"), "{}", run.stderr);
}
