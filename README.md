# search-headless

A single Rust binary for LLM coding agents that need web search and page content extraction via bash-callable commands.

One executable, no runtime to install. It answers to three names: `search-headless` with `search` and `fetch` subcommands, and the `search-web` / `fetch-content` symlinks that `install.sh` drops beside it.

## Commands

```bash
search-web "query"                              # Brave Search
search-web "query" --fetch --fetch-limit 5      # Search + fetch top results
fetch-content "https://example.com/article"     # Extract page content
fetch-content url-a url-b url-c                 # Fetch several pages in parallel

search-headless search "query" --limit 5        # the same thing, spelled out
search-headless fetch https://example.com
```

Default output is JSON on stdout. Diagnostics and human-readable progress go to stderr. Markdown output is available with `--format markdown`.

## Search Engines

| Engine | Backend | Challenge | Login |
|--------|---------|-----------|-------|
| **Brave** | Obscura | Reported as `blocked`, never solved | Not required |

Brave is the only engine. Obscura is the only browser — there is no Playwright, and no Chromium download.
Region: all, Language: English.

## Rate limiting

Brave searches are spaced 10 seconds apart to avoid triggering bot detection. The timestamp lives in
`/tmp/search-headless-last-search`, and the whole read-wait-stamp sequence runs under a lock file, so
two `search-web` processes started at the same moment queue up instead of both reading the same old
timestamp and firing together.

If the required wait is longer than the remaining `--timeout`, the command reports `timeout` rather
than blocking past the deadline it was given.

## Caching

Successful results are cached on disk, so a repeated query or a page an agent revisits costs nothing
and — more importantly — does not spend a slot against the rate limit.

| Namespace | Default TTL | Key |
|-----------|-------------|-----|
| `search` | 15 minutes | query + limit |
| `fetch` | 6 hours | URL + `--max-chars` |

Only `ok` results are stored: a `blocked` SERP or a failed fetch is never cached. Entries live under
`$XDG_CACHE_HOME/search-headless` (`~/.cache/search-headless`), are written atomically, and expired
ones are swept in the background as new entries are written.

```bash
search-web "query" --no-cache            # ignore and do not write the cache
search-web "query" --cache-ttl 60        # treat entries older than 60s as stale
rm -rf ~/.cache/search-headless          # drop everything
```

Set `SEARCH_HEADLESS_NO_CACHE=1` to disable it process-wide, or `SEARCH_HEADLESS_CACHE_DIR` to move it.

## Output Formats

Both commands support `--format json` (default) and `--format markdown`:

```bash
# JSON (default) — structured, machine-readable
search-web "query" --format json

# Markdown — human/LLM-friendly, readable
search-web "query" --format markdown
fetch-content "url" --format markdown
```

## Install on a machine

Prerequisites:

- `git`
- `cargo` (see [rustup.rs](https://rustup.rs))

Clone and install:

```bash
mkdir -p ~/dev
git clone git@github.com:jandrikus/search-headless.git ~/dev/search-headless
cd ~/dev/search-headless
./install.sh
```

`install.sh` runs:

- Installs `obscura` automatically if not found
- `cargo build --release`
- Copies the binary to `~/.local/bin` and symlinks `search-web` and `fetch-content` beside it

After installation, verify the commands are globally available:

```bash
search-web "bun typescript" --limit 5 --timeout 15000
fetch-content https://example.com --max-chars 500
```

If the commands are not found, put the install directory on PATH:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Shipping it elsewhere

The release binary is about 8 MB and links against nothing but `libc`, `libm` and `libgcc_s` - the
TLS stack and the CA roots are compiled in, so there is no OpenSSL to match and no certificate store
to find. Copying it to another machine of the same architecture with a glibc at least as new is
enough.

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

`search-headless` powers the [`pi-search-tool`](https://github.com/jandrikus/pi-extensions/tree/main/pi-search-tool) extension for [Pi Coding Agent](https://github.com/earendil-works/pi-coding-agent). The extension adds `search` and `fetch` tools to Pi, using the CLI commands installed above.

To use it, install this first (steps above), then add the extension to your Pi config:

```json
{
  "extensions": ["pi-search-tool"]
}
```

## Development commands

During development, run it straight out of the build directory:

```bash
cargo run -- search "bun typescript" --limit 3
cargo run -- fetch https://example.com https://bun.sh
```

## `search-web`

```bash
search-web "<query>" [options]
```

Options:

```text
--limit <n>              Number of search results. Default: 10.
--fetch                  Fetch content for top results.
--fetch-limit <n>        Number of result URLs to fetch. Default: 5.
--format <json|markdown> Output format. Default: json.
--timeout <ms>           Overall timeout. Default: 60000.
--concurrency <n>        Parallel content fetches for --fetch. Default: 5.
--no-cache               Ignore cached results and do not store new ones.
--cache-ttl <s>          Freshness window for cached results, in seconds.
```

## `fetch-content`

```bash
fetch-content "<url>" [<url>...] [options]
```

Options:

```text
--format <json|markdown> Output format. Default: json.
--timeout <ms>           Overall timeout for the whole run. Default: 60000.
--max-chars <n>          Truncate returned text. Default: 50000.
--concurrency <n>        URLs fetched in parallel. Default: 5.
--no-cache               Ignore cached results and do not store new ones.
--cache-ttl <s>          Freshness window for cached results, in seconds.
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

A search result page is handed back to Rust and parsed there, in `src/search/parse_brave.rs`,
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

## Challenges

If Brave serves a bot challenge, or any other interstitial in place of results, the command
reports `status: "blocked"` with a `BRAVE_CHALLENGE` error and exits non-zero. It never reports an
empty result set as success, and a challenge stays terminal — a later `--fetch` failure cannot
downgrade it to `partial`.

Challenges are reported, not solved. Nothing is clicked, and obscura has no layout or paint
engine, so there is no visible window and no human-in-the-loop path either.

## Safety boundary

This project does not implement CAPTCHA solving, browser fingerprint spoofing, proxy rotation, or automatic anti-bot circumvention. It runs obscura with `--stealth` and otherwise stops at any challenge and reports `blocked`.

## Private URL blocking

`fetch-content` rejects localhost and private network URLs by default, including common IPv4 private ranges, link-local addresses, loopback, and local IPv6 ranges. Test-only local server access uses `SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS=1` and is not a normal user-facing flag.

## Exit codes

- `0`: `ok` or `partial`
- `1`: `blocked`, `timeout`, or `error`
- `2`: bad arguments, before any work is done

## Environment variables

| Variable | Effect |
|----------|--------|
| `OBSCURA_BIN` | Path to the obscura executable. Default: `obscura` on PATH. |
| `SEARCH_HEADLESS_CACHE_DIR` | Where cached results live. Default: `$XDG_CACHE_HOME/search-headless`. |
| `SEARCH_HEADLESS_NO_CACHE` | `1` disables the cache entirely. |
| `SEARCH_HEADLESS_RATE_FILE` | Path of the rate-limit timestamp. Default: `/tmp/search-headless-last-search`. |
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

## License

Apache-2.0. See [LICENSE](./LICENSE).
