# search-headless

Bun + TypeScript CLI tools for LLM coding agents that need web search and page content extraction via bash-callable commands.

## Commands

```bash
search-web "query"
search-web "query" --fetch --fetch-limit 5
fetch-content "https://example.com/article"
```

Default output is JSON on stdout. Diagnostics and human-readable progress go to stderr. Markdown output is available with `--format markdown`.

## Install on a machine

Prerequisites:

- `git`
- `bun`
- SSH access to the private GitHub repository

Clone and install:

```bash
mkdir -p ~/dev
git clone git@github.com:jandrikus/search-web.git ~/dev/search-web
cd ~/dev/search-web
./install.sh
```

`install.sh` runs:

- `bun install`
- `bunx playwright install chromium`
- Checks for `obscura` (optional, used by `fetch-content --browser`)
- `bun link`

After installation, verify the commands are globally available:

```bash
search-web "bun typescript playwright" --limit 5 --timeout 15000
fetch-content https://example.com --max-chars 500
```

If the commands are not found, ensure Bun's global bin directory is on PATH:

```bash
export PATH="$HOME/.bun/bin:$PATH"
```

Add that line to your shell config, for example `~/.bashrc`, `~/.zshrc`, or equivalent.

## Add the skill to Pi Coding Agent

`install.sh` copies the skill into `~/.pi/agent/skills/search-fetch/` automatically. Pi discovers it on next start. To force-load:

```text
/skill:search-fetch
```

## Development commands

During development, you can still run commands directly:

```bash
bun run src/cli/search-web.ts "bun typescript" --limit 3 --no-human
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
--headful                Force visible browser.
--no-human               Fail fast if human intervention is needed.
--timeout <ms>           Overall timeout. Default: 60000.
```

`--region` is intentionally deferred for v0.1.

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

`fetch-content` uses `obscura fetch --stealth` to extract readable text, title, and metadata from any URL. Requires `obscura` on PATH.

## Human intervention

`search-web` uses Playwright Chromium with a persistent profile for Google search. If Google shows consent, CAPTCHA, or unusual-traffic pages, the tool opens a visible browser for manual challenge solving. With `--no-human`, `search-web` fails fast with structured JSON.

`fetch-content` uses `obscura` (a stateless headless browser) for the browser path and does not support human intervention.

## Safety boundary

This project does not implement CAPTCHA solving, stealth plugins, browser fingerprint spoofing, proxy rotation, or automatic anti-bot circumvention. It uses normal browser automation and graceful human-in-the-loop recovery.

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
