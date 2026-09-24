//! CLI regressions with a local stand-in for Obscura. No live Brave requests.
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const BINARY: &str = env!("CARGO_BIN_EXE_search-headless");

struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new(html: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let payload = serde_json::json!({
            "html": html, "text": html,
            "url": "https://search.brave.com/search?q=test",
        });
        std::fs::write(dir.path().join("page.json"), payload.to_string()).unwrap();
        let bin = dir.path().join("obscura");
        std::fs::write(&bin, format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{0}/calls'\ncase \"$2\" in *offset=*) if [ -f '{0}/page2.json' ]; then cat '{0}/page2.json'; exit; fi;; esac\ncat '{0}/page.json'\n",
            dir.path().display(),
        )).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn command(&self, query: &str) -> Command {
        let mut cmd = Command::new(BINARY);
        cmd.args(["search", query])
            .env("OBSCURA_BIN", self.dir.path().join("obscura"))
            .env("SEARCH_HEADLESS_STATE_DIR", self.dir.path().join("state"))
            .env("SEARCH_HEADLESS_CACHE_DIR", self.dir.path().join("cache"))
            .env("SEARCH_HEADLESS_RATE_FILE", self.dir.path().join("rate"))
            .env("SEARCH_HEADLESS_NO_RATE_LIMIT", "1")
            .env_remove("SEARCH_HEADLESS_BRAVE_FIXTURE")
            .env_remove("SEARCH_HEADLESS_BRAVE_FIXTURE_2");
        cmd
    }

    fn run(&self, query: &str) -> serde_json::Value {
        json(self.command(query).arg("--no-cache").output().unwrap())
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("calls")).unwrap_or_default()
    }
}

fn json(output: Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
}

#[test]
fn defaults_to_one_page_even_when_limit_exceeds_results() {
    let sandbox = Sandbox::new(&fixture("brave-results.html"));
    let result = sandbox.run("example");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["results"].as_array().unwrap().len(), 3);
    assert_eq!(sandbox.calls().lines().filter(|line| *line == "fetch").count(), 1);
}

#[test]
fn reuses_the_same_private_storage_directory_across_queries() {
    let sandbox = Sandbox::new(&fixture("brave-results.html"));
    sandbox.run("first");
    sandbox.run("second");
    let calls = sandbox.calls();
    let lines: Vec<_> = calls.lines().collect();
    let storage: Vec<_> = lines.windows(2).filter(|pair| pair[0] == "--storage-dir").map(|pair| pair[1]).collect();
    assert_eq!(storage.len(), 2);
    assert_eq!(storage[0], storage[1]);
    let mode = std::fs::metadata(storage[0]).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_challenge_stops_other_queries_across_cli_processes() {
    let sandbox = Sandbox::new(&fixture("brave-blocked.html"));
    let first = sandbox.run("first");
    assert_eq!(first["errors"][0]["code"], "BRAVE_CHALLENGE");
    let before = sandbox.calls();
    let second = sandbox.run("different query");
    assert_eq!(second["status"], "blocked");
    assert_eq!(second["errors"][0]["code"], "BRAVE_COOLDOWN");
    assert!(second["errors"][0]["details"]["retryAfterSeconds"].as_u64().unwrap() > 0);
    assert_eq!(sandbox.calls(), before);
}

#[test]
fn unknown_pages_and_javascript_notices_are_not_empty_successes_or_challenges() {
    for (html, code) in [
        ("<html><body><main>Loading...</main></body></html>", "BRAVE_UNRECOGNIZED_PAGE"),
        ("<html><body><h1>This page needs JavaScript to run.</h1></body></html>", "BRAVE_JAVASCRIPT_REQUIRED"),
        (
            "<html><body><div class='snippet' data-type='web'><a href='https://example.com'>Changed markup</a></div></body></html>",
            "BRAVE_PARSE_ERROR",
        ),
    ] {
        let sandbox = Sandbox::new(html);
        let result = sandbox.run("example");
        assert_eq!(result["status"], "error", "{result}");
        assert_eq!(result["errors"][0]["code"], code);
        assert!(result["errors"][0]["details"].is_object());
        assert_ne!(sandbox.run("another")["errors"][0]["code"], "BRAVE_COOLDOWN");
    }
}

#[test]
fn queued_processes_do_not_send_requests_after_the_first_challenge() {
    let sandbox = Sandbox::new(&fixture("brave-blocked.html"));
    let outputs = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|index| {
                let sandbox = &sandbox;
                scope.spawn(move || sandbox.run(&format!("query {index}")))
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(outputs.iter().filter(|result| result["errors"][0]["code"] == "BRAVE_CHALLENGE").count(), 1);
    assert_eq!(outputs.iter().filter(|result| result["errors"][0]["code"] == "BRAVE_COOLDOWN").count(), 3);
    assert_eq!(sandbox.calls().lines().filter(|line| *line == "fetch").count(), 1);
}

#[test]
fn an_expired_cooldown_allows_a_request_without_discarding_the_profile() {
    let sandbox = Sandbox::new(&fixture("brave-blocked.html"));
    sandbox.run("first");
    let state = sandbox.dir.path().join("state");
    std::fs::write(state.join("profile/marker"), "synthetic state").unwrap();
    std::fs::write(state.join("cooldown-until"), "0").unwrap();
    let next = sandbox.run("second");
    assert_eq!(next["errors"][0]["code"], "BRAVE_CHALLENGE");
    assert_eq!(sandbox.calls().lines().filter(|line| *line == "fetch").count(), 2);
    assert!(state.join("profile/marker").exists());
}

#[test]
fn a_busy_session_respects_the_deadline_and_releases_on_exit() {
    let sandbox = Sandbox::new(&fixture("brave-results.html"));
    let state = sandbox.dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let lock = std::fs::File::create(state.join("session.lock")).unwrap();
    lock.lock().unwrap();
    let result = json(sandbox.command("busy").args(["--timeout", "100"]).output().unwrap());
    assert_eq!(result["status"], "timeout");
    assert!(sandbox.calls().is_empty());
    drop(lock);
    assert_eq!(sandbox.run("free")["status"], "ok");
}

#[test]
fn a_fresh_cached_result_is_usable_during_cooldown() {
    let sandbox = Sandbox::new(&fixture("brave-results.html"));
    let first = json(sandbox.command("cached query").output().unwrap());
    assert_eq!(first["status"], "ok");
    std::fs::write(
        sandbox.dir.path().join("page.json"),
        serde_json::json!({
            "html": fixture("brave-blocked.html"), "text": "", "url": "https://search.brave.com/"
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(sandbox.run("blocked query")["status"], "blocked");
    let before = sandbox.calls();
    let cached = json(sandbox.command("cached query").output().unwrap());
    assert_eq!(cached["status"], "ok");
    assert_eq!(cached["cached"], true);
    assert_eq!(sandbox.calls(), before);
}

#[test]
fn a_pagination_parse_failure_preserves_results_without_caching_the_failure() {
    let sandbox = Sandbox::new(&fixture("brave-results.html"));
    std::fs::write(sandbox.dir.path().join("page2.json"), serde_json::json!({
        "html": "<html><body>Loading...</body></html>", "text": "", "url": "https://search.brave.com/search?offset=1"
    }).to_string()).unwrap();
    for _ in 0..2 {
        let result = json(sandbox.command("same query").arg("--paginate").output().unwrap());
        assert_eq!(result["status"], "partial");
        assert_eq!(result["results"].as_array().unwrap().len(), 3);
        assert_eq!(result["errors"][0]["code"], "BRAVE_UNRECOGNIZED_PAGE");
        assert!(result["cached"].is_null());
    }
    assert_eq!(sandbox.calls().lines().filter(|line| *line == "fetch").count(), 4);
}

#[test]
fn invalid_extraction_output_is_not_reported_as_a_network_failure() {
    let sandbox = Sandbox::new("");
    std::fs::write(sandbox.dir.path().join("page.json"), "{}").unwrap();
    let result = sandbox.run("example");
    assert_eq!(result["status"], "error");
    assert_eq!(result["errors"][0]["code"], "OBSCURA_EVAL_FAILED");
}

#[test]
fn corrupted_state_fails_without_sending_a_request() {
    let sandbox = Sandbox::new(&fixture("brave-results.html"));
    let state = sandbox.dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("cooldown-until"), "invalid").unwrap();
    let result = sandbox.run("example");
    assert_eq!(result["status"], "error");
    assert_eq!(result["errors"][0]["code"], "BRAVE_STATE_ERROR");
    assert!(sandbox.calls().is_empty());
}

#[test]
fn pagination_has_a_separate_cache_key() {
    let sandbox = Sandbox::new(&fixture("brave-results.html"));
    json(sandbox.command("same query").output().unwrap());
    let before = sandbox.calls();
    let result = json(sandbox.command("same query").arg("--paginate").output().unwrap());
    assert_eq!(result["status"], "ok");
    assert!(result["cached"].is_null());
    assert_ne!(sandbox.calls(), before);
}

#[test]
fn explicit_no_results_is_a_success() {
    let sandbox =
        Sandbox::new("<html><body><main><h1>No results found</h1><p>Try fewer keywords</p></main></body></html>");
    let result = sandbox.run("example");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["results"].as_array().unwrap().len(), 0);
    assert_eq!(result["errors"].as_array().unwrap().len(), 0);
}
