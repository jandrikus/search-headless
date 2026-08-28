# search-headless

Bun + TypeScript CLI tools for LLM coding agents that need web search and page content extraction via bash-callable commands.

## Commands

```bash
search-web "query"                              # Brave Search (default)
search-web "query" --fetch --fetch-limit 5      # Search + fetch top results
fetch-content "https://example.com/article"     # Extract page content
```

Default output is JSON on stdout. Diagnostics and human-readable progress go to stderr. Markdown output is available with `--format markdown`.

## Search Engines

| Engine | Backend | Challenge | Login |
|--------|---------|-----------|-------|
| **Brave** | Obscura | Reported as `blocked`, never solved | Not required |

Brave is the only engine. Obscura is the only browser — there is no Playwright, and no Chromium download.
Region: all, Language: English.

## Rate Limiting

Brave searches are automatically spaced 10 seconds apart to avoid triggering bot detection. The timestamp is stored in `/tmp/search-headless-last-search` (RAM-based for speed). No configuration needed — it just works.

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
- `bun` (recommended) or `npm`

Clone and install:

```bash
mkdir -p ~/dev
git clone git@github.com:jandrikus/search-headless.git ~/dev/search-headless
cd ~/dev/search-headless
./install.sh
```

`install.sh` runs:

- Detects `bun` or `npm` (prefers bun, falls back to npm)
- Installs `obscura` automatically if not found
- Installs dependencies
- Links commands globally (`bun link` / `npm link`)

After installation, verify the commands are globally available:

```bash
search-web "bun typescript" --limit 5 --timeout 15000
fetch-content https://example.com --max-chars 500
```

If the commands are not found, ensure your global bin directory is on PATH:

```bash
# For bun
export PATH="$HOME/.bun/bin:$PATH"
# For npm
export PATH="$(npm bin -g):$PATH"
```

## Pi Extension

`search-headless` powers the [`pi-search-tool`](https://github.com/jandrikus/pi-extensions/tree/main/pi-search-tool) extension for [Pi Coding Agent](https://github.com/earendil-works/pi-coding-agent). The extension adds `search` and `fetch` tools to Pi, using the CLI commands installed above.

To use it, install this first (steps above), then add the extension to your Pi config:

```json
{
  "extensions": ["pi-search-tool"]
}
```

## Development commands

During development, you can still run commands directly:

```bash
bun run src/cli/search-web.ts "bun typescript" --limit 3
bun run src/cli/search-web.ts "bun typescript" --limit 3
bun run src/cli/fetch-content.ts https://example.com
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
```

## `fetch-content`

```bash
fetch-content "<url>" [options]
```

Options:

```text
--format <json|markdown> Output format. Default: json.
--timeout <ms>           Overall timeout. Default: 60000.
--max-chars <n>          Truncate returned text. Default: 50000.
```

`fetch-content` extracts in two tiers. Plain HTTP + Readability runs first: it is cheaper and gives better text, plus byline, excerpt and published time. Obscura runs second, for pages whose content is rendered by JavaScript. Requires `obscura` on PATH.

## How obscura is driven

Every obscura invocation goes through one wrapper, `src/obscura/runner.ts`, so argument
construction and deadline clamping stay in a single place. Each fetch is its own `obscura fetch`
process, which parallelizes better than a shared daemon would: obscura serializes page work
within a process, so N processes genuinely beat one.

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
- non-zero: `blocked`, `timeout`, or `error`

## Development

```bash
bun run typecheck
bun test
```

## License

Apache-2.0. See [LICENSE](./LICENSE).
