# search-headless

A single Rust binary for LLM coding agents that need web search and page content extraction via bash-callable commands.

One executable, no runtime to install, two subcommands: `search` and `fetch`.

## Commands

```bash
search-headless search "query"                           # Brave Search, the CLI default
search-headless search "query" --engine searxng          # Configured SearXNG instance
search-headless search "query" --fetch --fetch-limit 5   # Search + fetch top results
search-headless fetch "https://example.com/article"      # Extract page content
search-headless fetch url-a url-b url-c                  # Fetch several pages in parallel
```

Default output is JSON on stdout. Diagnostics and human-readable progress go to stderr. Markdown output is available with `--format markdown`.

## Search engines

| Engine | How it searches | Authentication |
|--------|-----------------|----------------|
| Brave | Loads Brave Search through Obscura | None |
| SearXNG | Requests JSON from a configured SearXNG instance | HTTP Basic, supplied through environment variables |

The CLI defaults to Brave.
Select SearXNG explicitly with `--engine searxng`.
The Pi extension has its own default: SearXNG.
Neither backend automatically falls back to the other.
Brave uses region `all` and English.
Obscura is the only browser backend; there is no Playwright or Chromium download.

### SearXNG setup

Configure an HTTPS SearXNG base URL with `SEARXNG_URL` or `--searxng-url`.
The option takes precedence over the environment variable.
The URL must not contain credentials, a query string or a fragment.
Use the final base URL, because redirects are not followed for authenticated requests.
The instance must support JSON output through `search.formats` in its `settings.yml`.

Set `SEARXNG_USER` and `SEARXNG_PASS` in the environment of the process that runs the command.
Both must be nonempty, even if the instance is public, and the username must not contain a colon.
Do not put credentials in the URL or Pi settings.

```bash
# Set SEARXNG_USER and SEARXNG_PASS in your environment first.
export SEARXNG_URL="https://your-searxng.example"
search-headless search "rust async" --engine searxng --limit 10
search-headless search "rust async" --engine searxng --limit 20 --paginate
```

If an upstream SearXNG engine fails, usable results are returned with `partial` status and a `SEARXNG_ENGINE_ERROR` warning.
The SearXNG credentials are not sent to result URLs when using `--fetch`.
A SearXNG failure does not trigger a Brave search.

## Rate limiting

Brave searches are spaced 10 seconds apart to avoid triggering bot detection.
This limit does not apply to SearXNG, which may have its own rate limits.
The timestamp lives in the private Brave state directory, and the whole read-wait-stamp sequence runs under a lock file, so
two `search` processes started at the same moment queue up instead of both reading the same old
timestamp and firing together.

If the required wait is longer than the remaining `--timeout`, the command reports `timeout` rather
than blocking past the deadline it was given.

## Caching

Successful results are cached on disk, so a repeated query or a page an agent revisits costs nothing
and — more importantly — does not spend a slot against the rate limit.

| Namespace | Default TTL | Key |
|-----------|-------------|-----|
| `search` | 15 minutes | Brave query, limit and pagination option |
| `searxng` | 15 minutes | SearXNG endpoint, username, query, limit and pagination option |
| `fetch` | 6 hours | URL + `--max-chars` |

Only `ok` results are stored: a `blocked` SERP or a failed fetch is never cached. Entries live under
`$XDG_CACHE_HOME/search-headless` (`~/.cache/search-headless`), are written atomically, and expired
ones are swept in the background as new entries are written.
Cache entries may contain search queries, URLs and page text.
Cache namespace directories use mode `0700` and new entries use mode `0600`.
Existing entries retain their previous file permissions until replaced.

```bash
search-headless search "query" --no-cache      # ignore and do not write the cache
search-headless search "query" --cache-ttl 60  # new cache entries expire after 60s
rm -rf ~/.cache/search-headless          # drop everything
```

Set `SEARCH_HEADLESS_NO_CACHE=1` to disable it process-wide, or `SEARCH_HEADLESS_CACHE_DIR` to move it.
The TTL is stored when an entry is written; changing `--cache-ttl` does not shorten the life of an existing entry.
Use `--no-cache` to bypass existing entries.

## Output Formats

Both subcommands support `--format json` (default) and `--format markdown`:

```bash
# JSON (default) — structured, machine-readable
search-headless search "query" --format json

# Markdown — human/LLM-friendly, readable
search-headless search "query" --format markdown
search-headless fetch "url" --format markdown
```

## Install on a machine

### Prebuilt release

After the first `v0.1.0` release is published, Apple Silicon macOS and x86-64 Linux users can install without Rust:

```bash
curl -fLO https://github.com/jandrikus/search-headless/releases/download/v0.1.0/install.sh
less install.sh  # review the installer before running it
bash install.sh --binary
```

The Linux binary is built on Ubuntu 24.04 and requires a compatible glibc.
The installer verifies the CLI archive against the checksum published alongside it, and verifies Obscura against the digest pinned in the installer.
A checksum from the same release detects corruption but does not independently authenticate that release.

### Build from source

Prerequisites are `git` and `cargo` (see [rustup.rs](https://rustup.rs)).
Clone and install:

```bash
mkdir -p ~/dev
git clone https://github.com/jandrikus/search-headless.git ~/dev/search-headless
cd ~/dev/search-headless
bash install.sh
```

The source installer installs Obscura if needed, runs `cargo build --locked --release`, and copies the CLI to `~/.local/bin`.

The installer pins Obscura `v0.2.6` and verifies the archive against the digest recorded in the script before unpacking it.
It selects the stealth build because the browser calls use `--stealth`.
For another version, set both `OBSCURA_VERSION` and `OBSCURA_SHA256`.
If Obscura is already on PATH, the installer checks its version and does not replace it.
The version check does not verify an existing executable's provenance or build features.

After installation, verify the command is globally available:

```bash
search-headless search "rust async" --limit 5 --timeout 15000
search-headless fetch https://example.com --max-chars 500
```

If it is not found, put the install directory on PATH in the current shell:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

To make this persistent in macOS's default zsh shell, add that line to `~/.zshrc` and start a new shell.

### Shipping it elsewhere

A Linux build links against the system C runtime.
For a dynamically linked build, the target machine needs a compatible architecture and glibc version.

For a fully static binary that ignores glibc versions entirely, build against musl. The TLS backend
compiles C, so this needs a musl C toolchain as well as the Rust target:

```bash
# Arch: pacman -S musl   Debian/Ubuntu: apt install musl-tools
rustup target add x86_64-unknown-linux-musl
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --release --target x86_64-unknown-linux-musl
```

`obscura` stays a separate executable either way: it embeds V8 and publishes no library crate, so it
cannot be linked in. Ship the two files together.

## Pi Extension

`search-headless` powers the [`pi-search-tool`](https://github.com/jandrikus/pi-search-tool) extension for [Pi Coding Agent](https://github.com/earendil-works/pi-coding-agent). The extension adds `search` and `fetch` tools to Pi, shelling out to the CLI installed above. It calls `search-headless search` and `search-headless fetch`; versions of the extension older than the Rust rewrite invoked `search-web` and `fetch-content` and will not work.

Install `search-headless` as described above, then install the extension using its [installation instructions](https://github.com/jandrikus/pi-search-tool).
For SearXNG, add the following to Pi's user settings:

```json
{
  "searchTool": {
    "engine": "searxng",
    "searxngUrl": "https://your-searxng.example"
  }
}
```

The extension reads `searchTool` from Pi's user settings, not project settings.
It defaults to `searxng` and uses `SEARXNG_URL` if `searchTool.searxngUrl` is absent.
Set `SEARXNG_USER` and `SEARXNG_PASS` in the environment that starts Pi.
Set `searchTool.engine` to `brave` to use Brave instead.
See the [extension README](https://github.com/jandrikus/pi-search-tool) for its tool parameters and installation options.

## Development commands

During development, run it straight out of the build directory:

```bash
cargo run -- search "bun typescript" --limit 3
cargo run -- fetch https://example.com https://bun.sh
```

## `search-headless search`

```bash
search-headless search "<query>" [options]
```

Options:

```text
--engine <brave|searxng> Search backend. Default: brave.
--searxng-url <url>      SearXNG base URL. Defaults to SEARXNG_URL.
--limit <n>              Maximum results. Default: 10.
--paginate               Follow additional pages until the limit is reached.
--fetch                  Fetch content for top results.
--fetch-limit <n>        Number of result URLs to fetch. Default: 5.
--format <json|markdown> Output format. Default: json.
--timeout <ms>           Overall timeout. Default: 60000.
--concurrency <n>        Parallel content fetches for --fetch. Default: 5.
--no-cache               Ignore cached results and do not store new ones.
--cache-ttl <s>          Lifetime of newly cached results, in seconds.
```

## `search-headless fetch`

```bash
search-headless fetch "<url>" [<url>...] [options]
```

Options:

```text
--format <json|markdown> Output format. Default: json.
--timeout <ms>           Overall timeout for the whole run. Default: 60000.
--max-chars <n>          Truncate returned text. Default: 50000.
--concurrency <n>        URLs fetched in parallel. Default: 5.
--no-cache               Ignore cached results and do not store new ones.
--cache-ttl <s>          Lifetime of newly cached results, in seconds.
```

One URL returns the single content object callers already parse. Several URLs return a batch envelope
— `{ status, results, errors }` — so a page that fails stays attached to its own URL instead of
disappearing from the list. A batch where at least one URL worked is `partial` and exits `0`.

### Tiers

Extraction runs in two tiers, cheapest first.

1. **HTTP + Readability.** No browser. Yields separated paragraphs plus title, byline, excerpt,
   site name and published time. Requests are sent with ordinary browser headers, because a default
   HTTP client agent string is answered with a 403 by a large share of sites.
2. **Obscura.** A real render, for pages whose content is written by script. Requires `obscura` on PATH.

A thin HTTP result on a script-bearing page is treated as a possibly unrendered shell and re-checked
against tier 2; whichever tier saw more text wins. If tier 2 cannot run, a usable tier 1 result is
still returned rather than thrown away.

Tier 2 is skipped where it cannot help: a PDF stays a PDF, an oversized body stays oversized, and a
404 or a 500 is the same page in any client — rendering those would just hand back the error page's
own text as if it were the article. A 401, 403, 405, 406, 429 or 451 does escalate, because those are
the responses a real browser is plausibly served differently.

## How obscura is driven

Every obscura invocation goes through one wrapper, `src/obscura.rs`, so argument
construction and deadline clamping stay in a single place. Each fetch is its own `obscura fetch`
process, which parallelizes better than a shared daemon would: obscura serializes page work
within a process, so N processes genuinely beat one.

A Brave search result page is handed back to Rust and parsed there, in `src/search/parse_brave.rs`,
rather than picked apart by a script running inside the page. That keeps the parser under the fixture
tests: the code that runs in production is the code the tests exercise. Only inert weight — script,
style, svg, iframe and friends — is dropped before the HTML crosses the pipe, which takes a live
Brave SERP from about 139 KB to about 7 KB. The footer stays, because Brave's "Next" link lives
there, and `noscript` stays, because a JavaScript-required notice is one of the interstitials that
has to be detected.

A page load that fails for any reason other than a challenge is retried once. A challenge is never
retried, and a browser that will not start is reported as `OBSCURA_SEARCH_FAILED`, never as
`blocked` — an agent should not go and wait out a block that does not exist.

Point `OBSCURA_BIN` at another executable to run the suite against a stand-in binary.

## Brave challenges and session state

If Brave serves a bot challenge, the command reports `status: "blocked"` with a `BRAVE_CHALLENGE` error and exits non-zero.
A challenge stays terminal even if earlier pages returned results.
Other unrecognized interstitials are reported as errors rather than successful empty results.

Brave searches share a persistent Obscura profile and a 15-minute cooldown after a challenge.
During the cooldown, new uncached Brave searches return `BRAVE_COOLDOWN` without sending a request; fresh cached results remain available.
The state lives in `$XDG_STATE_HOME/search-headless/brave`, or `~/.local/state/search-headless/brave` when `XDG_STATE_HOME` is not set.
`SEARCH_HEADLESS_STATE_DIR` overrides that directory.
SearXNG is not subject to the Brave cooldown.

Challenges are reported, not solved.
Nothing is clicked and this tool does not offer a visible window or human-in-the-loop path.

## Safety boundary

This project does not solve CAPTCHAs or rotate proxies.
It invokes Obscura with `--stealth`, which enables Obscura's fingerprint randomization and tracker blocking in its stealth build.
A detected challenge is reported as `blocked` and is not solved.

## Private URL blocking

`fetch` rejects localhost and private network URLs by default, including common IPv4 private ranges, link-local addresses, loopback, and local IPv6 ranges.
The HTTP tier connects to the addresses it checked and does not use system proxies.
The browser tier relies on Obscura's private-network protection.
These checks are defense in depth, not a sandbox for running untrusted web content.
Test-only local server access uses `SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS=1` and is not a normal user-facing flag.

## Exit codes

- `0`: `ok` or `partial`
- `1`: `blocked`, `timeout`, or `error`
- `2`: bad arguments, before any work is done

## Environment variables

| Variable | Effect |
|----------|--------|
| `OBSCURA_BIN` | Path to the obscura executable. Default: `obscura` on PATH. |
| `OBSCURA_VERSION` | `install.sh` only. Obscura release tag. Default: `v0.2.6`. |
| `OBSCURA_SHA256` | `install.sh` only. Expected SHA256 of the stealth archive. Required for a different version. |
| `SEARXNG_URL` | HTTPS base URL of the SearXNG instance, unless overridden by `--searxng-url`. |
| `SEARXNG_USER` | HTTP Basic username for SearXNG. |
| `SEARXNG_PASS` | HTTP Basic password for SearXNG. |
| `SEARCH_HEADLESS_STATE_DIR` | Persistent Brave profile and cooldown directory. |
| `SEARCH_HEADLESS_CACHE_DIR` | Where cached results live. Default: `$XDG_CACHE_HOME/search-headless`. |
| `SEARCH_HEADLESS_NO_CACHE` | `1` disables the cache entirely. |
| `SEARCH_HEADLESS_RATE_FILE` | Path of the rate-limit timestamp. Default: `last-search` in the Brave state directory. |
| `SEARCH_HEADLESS_NO_RATE_LIMIT` | `1` removes the 10s spacing between searches. |
| `SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS` | `1` permits localhost and private-network URLs. Tests only. |

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

Two tests drive the real `obscura` binary - one for each script that runs inside the page - and skip
themselves when it is not installed. Everything else uses a stand-in binary or a local HTTP server,
so the suite needs no network.

## How this was built

The code here was written by an AI coding agent. The thinking was not.

Worth knowing if you are reading the source or considering a patch: the comments
explain reasoning that was argued out rather than discovered by the code, and the
test suite is deliberately built around the failures that actually happened, not
around coverage.

## License

Apache-2.0. See [LICENSE](./LICENSE).
