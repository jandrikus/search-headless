# search-headless

Bun + TypeScript CLI tools for LLM coding agents that need web search and page content extraction via bash-callable commands.

## Commands

```bash
search-web "query"
search-web "query" --fetch --fetch-limit 5
fetch-content "https://example.com/article"
```

Default output is JSON on stdout. Diagnostics and human-readable progress go to stderr. Markdown output is available with `--format markdown`.

## Install

```bash
bun install
bunx playwright install chromium
```

During development, run commands directly:

```bash
bun run src/cli/search-web.ts "bun typescript" --limit 3 --no-human
bun run src/cli/fetch-content.ts https://example.com --no-browser
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
--browser                Force browser extraction.
--no-browser             Disable browser fallback.
--headful                Force visible browser.
--no-human               Reserved for challenge-sensitive flows.
--timeout <ms>           Overall timeout. Default: 60000.
--max-chars <n>          Truncate returned text. Default: 50000.
```

## Human intervention

If Google shows consent, CAPTCHA, or unusual-traffic pages and human intervention is enabled, the tool uses a visible browser path so a human can solve the challenge manually. With `--no-human`, commands fail fast with structured JSON.

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
