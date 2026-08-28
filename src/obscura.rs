//! Single place where `obscura` is invoked.
//!
//! Every call site goes through here so argument construction, deadline clamping
//! and failure reporting stay consistent, and so tests can point `bin` at a fake
//! binary and assert the arguments without needing obscura installed.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::process::Command;

use crate::types::Deadline;

/// Upper bound for `--wait`, in seconds.
///
/// `--wait` is how long obscura settles the page after load, so scaling it to the
/// whole remaining deadline would make a generous timeout translate into a minute
/// of idling per page.
const MAX_WAIT_SECS: u64 = 8;

#[derive(Debug, Clone)]
pub struct ObscuraFetchArgs {
    pub url: String,
    pub deadline: Deadline,
    /// JS evaluated in the page; its return value becomes stdout.
    pub eval_script: Option<String>,
    pub stealth: bool,
    pub wait_secs: Option<u64>,
    pub allow_private_network: bool,
    /// Override the binary. `None` reads `OBSCURA_BIN`, then falls back to PATH.
    pub bin: Option<PathBuf>,
}

impl ObscuraFetchArgs {
    pub fn new(url: impl Into<String>, deadline: Deadline) -> Self {
        Self {
            url: url.into(),
            deadline,
            eval_script: None,
            stealth: false,
            wait_secs: None,
            allow_private_network: false,
            bin: None,
        }
    }

    pub fn eval(mut self, script: impl Into<String>) -> Self {
        self.eval_script = Some(script.into());
        self
    }

    pub fn stealth(mut self, stealth: bool) -> Self {
        self.stealth = stealth;
        self
    }

    pub fn allow_private_network(mut self, allow: bool) -> Self {
        self.allow_private_network = allow;
        self
    }
}

#[derive(Debug)]
pub struct ObscuraFetchResult {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Resolve the obscura binary, honouring the environment override.
pub fn obscura_bin() -> PathBuf {
    match std::env::var("OBSCURA_BIN") {
        Ok(bin) if !bin.is_empty() => PathBuf::from(bin),
        _ => PathBuf::from("obscura"),
    }
}

/// Seconds obscura should settle the page, clamped to the remaining deadline.
pub fn resolve_wait_secs(deadline: &Deadline, requested: Option<u64>) -> u64 {
    let budget = deadline.remaining_ms() / 1000;
    let wanted = requested.unwrap_or(MAX_WAIT_SECS);
    wanted.min(MAX_WAIT_SECS).min(budget.max(1)).max(1)
}

pub fn build_obscura_args(args: &ObscuraFetchArgs) -> Vec<String> {
    let timeout_secs = args.deadline.remaining_ms().div_ceil(1000).max(1);
    let bin = args.bin.clone().unwrap_or_else(obscura_bin);

    let mut cmd = vec![
        bin.to_string_lossy().into_owned(),
        "fetch".to_string(),
        args.url.clone(),
        "--quiet".to_string(),
    ];
    if args.stealth {
        cmd.push("--stealth".to_string());
    }
    cmd.push("--wait".to_string());
    cmd.push(resolve_wait_secs(&args.deadline, args.wait_secs).to_string());
    cmd.push("--timeout".to_string());
    cmd.push(timeout_secs.to_string());
    if args.allow_private_network {
        cmd.push("--allow-private-network".to_string());
    }
    if let Some(script) = &args.eval_script {
        cmd.push("--eval".to_string());
        cmd.push(script.clone());
    }
    cmd
}

/// Run `obscura fetch` once.
///
/// Never fails on a non-zero exit; the caller inspects the result. A missing
/// binary surfaces as `exit_code: -1` with the spawn message on stderr.
pub async fn obscura_fetch(args: &ObscuraFetchArgs) -> ObscuraFetchResult {
    let timeout = Duration::from_millis(args.deadline.remaining_ms().max(1000));
    let argv = build_obscura_args(args);

    let child = match spawn(&argv).await {
        Ok(child) => child,
        Err(e) => return spawn_failure(e.to_string()),
    };

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        // The child is killed on drop, so a timeout does not leak a browser.
        Ok(Err(e)) => return spawn_failure(e.to_string()),
        Err(_) => return spawn_failure("obscura timed out.".to_string()),
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().unwrap_or(-1);

    ObscuraFetchResult { ok: exit_code == 0 && !stdout.trim().is_empty(), stdout, stderr, exit_code }
}

/// Spawn the child, retrying briefly on `ETXTBSY`.
///
/// A binary that is being written at this instant - by `install.sh` replacing
/// obscura, or by a test laying down a stand-in - cannot be executed until the
/// writer closes it, and the window is measured in milliseconds.
async fn spawn(argv: &[String]) -> std::io::Result<tokio::process::Child> {
    const RETRIES: usize = 5;

    for attempt in 0..RETRIES {
        let result = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        match result {
            Err(e) if e.raw_os_error() == Some(libc_etxtbsy()) && attempt + 1 < RETRIES => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => return other,
        }
    }
    unreachable!("the loop returns on its last attempt")
}

/// `ETXTBSY`, without taking a dependency on libc for one integer.
fn libc_etxtbsy() -> i32 {
    26
}

fn spawn_failure(message: String) -> ObscuraFetchResult {
    ObscuraFetchResult { ok: false, stdout: String::new(), stderr: message, exit_code: -1 }
}

/// Parse obscura's stdout as a JSON object.
///
/// `--eval` prints whatever the script returned, so a script that throws yields
/// empty or non-JSON output rather than an error exit. Returns `None` in that
/// case, and for any JSON that is not an object.
pub fn parse_obscura_json<T: DeserializeOwned>(stdout: &str) -> Option<T> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    if !value.is_object() {
        return None;
    }
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in obscura that records its argv and prints `emit` on stdout.
    fn fake_obscura(emit: &str, exit_code: i32) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = dir.path().join("argv.txt");
        let bin = dir.path().join("fake-obscura");
        let script = format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > {capture:?}\nprintf '%s' {emit:?}\nexit {exit_code}\n"
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, bin, capture)
    }

    #[test]
    fn clamps_wait_so_a_long_deadline_does_not_idle_the_page() {
        // The original code passed ceil(timeout/1000), so a 60s deadline meant --wait 60.
        let wait = resolve_wait_secs(&Deadline::new(60_000), None);
        assert!(wait <= MAX_WAIT_SECS);
        assert!(wait > 0);
    }

    #[test]
    fn clamps_wait_down_to_a_short_remaining_deadline() {
        assert!(resolve_wait_secs(&Deadline::new(2_000), None) <= 2);
    }

    #[test]
    fn builds_the_expected_argv() {
        let args = ObscuraFetchArgs::new("https://example.com", Deadline::new(30_000))
            .stealth(true)
            .eval("1+1");
        let argv = build_obscura_args(&args);

        assert_eq!(argv[1], "fetch");
        assert_eq!(argv[2], "https://example.com");
        assert!(argv.iter().any(|a| a == "--quiet"));
        assert!(argv.iter().any(|a| a == "--stealth"));
        assert!(argv.iter().any(|a| a == "--timeout"));
        let eval_at = argv.iter().position(|a| a == "--eval").expect("--eval");
        assert_eq!(argv[eval_at + 1], "1+1");
    }

    #[test]
    fn omits_optional_flags_that_were_not_requested() {
        let argv = build_obscura_args(&ObscuraFetchArgs::new("https://example.com", Deadline::new(30_000)));
        assert!(!argv.iter().any(|a| a == "--stealth"));
        assert!(!argv.iter().any(|a| a == "--eval"));
        assert!(!argv.iter().any(|a| a == "--allow-private-network"));
    }

    #[test]
    fn passes_allow_private_network_only_when_asked() {
        let args = ObscuraFetchArgs::new("http://127.0.0.1:1", Deadline::new(30_000)).allow_private_network(true);
        assert!(build_obscura_args(&args).iter().any(|a| a == "--allow-private-network"));
    }

    #[tokio::test]
    async fn runs_the_binary_and_hands_back_its_stdout() {
        let (_guard, bin, capture) = fake_obscura(r#"{"ok":true}"#, 0);
        let mut args = ObscuraFetchArgs::new("https://example.com", Deadline::new(10_000)).stealth(true);
        args.bin = Some(bin);

        let result = obscura_fetch(&args).await;
        assert!(result.ok, "{}", result.stderr);
        assert_eq!(result.stdout, r#"{"ok":true}"#);

        let argv = std::fs::read_to_string(capture).unwrap();
        assert!(argv.lines().any(|a| a == "--stealth"));
        assert!(argv.lines().any(|a| a == "https://example.com"));
    }

    #[tokio::test]
    async fn reports_a_non_zero_exit_without_failing() {
        let (_guard, bin, _capture) = fake_obscura("", 3);
        let mut args = ObscuraFetchArgs::new("https://example.com", Deadline::new(10_000));
        args.bin = Some(bin);

        let result = obscura_fetch(&args).await;
        assert!(!result.ok);
        assert_eq!(result.exit_code, 3);
    }

    #[tokio::test]
    async fn reports_a_missing_binary_rather_than_failing() {
        let mut args = ObscuraFetchArgs::new("https://example.com", Deadline::new(10_000));
        args.bin = Some(PathBuf::from("/nonexistent/obscura-does-not-exist"));

        let result = obscura_fetch(&args).await;
        assert!(!result.ok);
        assert_eq!(result.exit_code, -1);
    }

    #[test]
    fn treats_empty_or_non_json_output_as_no_result() {
        // `--eval` prints whatever the script returned, so a throwing script exits 0 with junk.
        assert!(parse_obscura_json::<serde_json::Value>("").is_none());
        assert!(parse_obscura_json::<serde_json::Value>("not json").is_none());
        assert!(parse_obscura_json::<serde_json::Value>("null").is_none());
        assert!(parse_obscura_json::<serde_json::Value>("42").is_none());

        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct A {
            a: u32,
        }
        assert_eq!(parse_obscura_json::<A>(r#"{"a":1}"#), Some(A { a: 1 }));
    }
}
