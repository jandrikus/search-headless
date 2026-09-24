//! Verify storage persistence with two real Obscura processes and a local site.
use search_headless::obscura::{ObscuraFetchArgs, obscura_fetch, parse_obscura_json};
use search_headless::testing;
use search_headless::types::Deadline;

#[tokio::test(flavor = "multi_thread")]
async fn cookies_survive_between_browser_processes() {
    let Some(bin) = testing::installed_obscura() else {
        eprintln!("skipping: obscura is not installed");
        return;
    };
    let server = testing::serve(|request| {
        if request.uri().path() == "/set" {
            let mut response = testing::html("<html><body>Set</body></html>");
            response
                .headers_mut()
                .insert("set-cookie", "test-cookie=synthetic-value; Path=/; Max-Age=3600; HttpOnly".parse().unwrap());
            response
        } else {
            let sent = request
                .headers()
                .get("cookie")
                .and_then(|header| header.to_str().ok())
                .is_some_and(|value| value.contains("test-cookie=synthetic-value"));
            testing::html(format!("<html><body><p id='cookie'>{sent}</p></body></html>"))
        }
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    for route in ["set", "check"] {
        let mut args = ObscuraFetchArgs::new(format!("{}/{route}", server.base_url), Deadline::new(30_000))
            .allow_private_network(true)
            .eval("JSON.stringify({cookie:document.getElementById('cookie')?.textContent})");
        args.bin = Some(bin.clone());
        args.storage_dir = Some(dir.path().join("profile"));
        args.wait_secs = Some(1);
        let result = obscura_fetch(&args).await;
        assert!(result.ok, "{}", result.stderr);
        let payload = parse_obscura_json::<serde_json::Value>(&result.stdout).unwrap();
        if route == "check" {
            assert_eq!(payload["cookie"], "true");
        }
    }
}
