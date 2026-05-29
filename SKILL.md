---
name: search-fetch
description: Use when the agent needs up-to-date knowledge beyond its training cutoff, when the user provides a URL to fetch, or when the agent must search the web to answer a question.
---

# Search & Fetch

Two CLI commands: `search-web` for web search (Brave by default), `fetch-content` for page extraction. Both use Obscura headless browser internally. JSON on stdout, diagnostics on stderr.

## Content Extraction Fallback

`fetch-content` uses a three-tier fallback strategy:

1. **URL Rewrite** — Domain-specific rewrites (e.g., `www.reddit.com` → `old.reddit.com`)
2. **Obscura** — Fast, lightweight headless browser (default)
3. **HTTP + Readability** — Plain HTTP request with Mozilla Readability extraction
4. **Playwright** — Full Chromium browser (last resort, slower)

Each tier falls back to the next when it returns empty text or an error. Reddit URLs are automatically rewritten, so no manual intervention is needed.

## Commands

### search-web — Search the web

```bash
search-web "query" --limit 5 --timeout 15000
```

Returns JSON:

```json
{
  "query": "query",
  "engine": "brave",
  "status": "ok",
  "results": [
    {
      "rank": 1,
      "title": "Result title",
      "url": "https://example.com",
      "displayUrl": "example.com",
      "snippet": "Optional snippet"
    }
  ],
  "errors": []
}
```

Options:

```text
--limit <n>              Number of results (default: 10)
--fetch                  Fetch content for top results
--fetch-limit <n>        Number of URLs to fetch when --fetch is set (default: 5)
--format <json|markdown> Output format (default: json)
--google                 Use Google instead of Brave (requires browser profile)
--headful                Force visible browser (Google only, for CAPTCHA solving)
--no-human               Fail fast if blocked (Google only)
--timeout <ms>           Overall timeout (default: 60000)
```

### fetch-content — Extract page content

```bash
fetch-content "https://example.com/article"
```

Returns JSON:

```json
{
  "url": "https://example.com/article",
  "status": "ok",
  "title": "Article title",
  "text": "Extracted page text...",
  "method": "browser",
  "truncated": false,
  "textLength": 12345,
  "errors": []
}
```

Options:

```text
--format <json|markdown> Output format (default: json)
--timeout <ms>           Overall timeout (default: 60000)
--max-chars <n>          Truncate text (default: 50000)
```

## Workflow

1. Search:

   ```bash
   search-web "specific query" --limit 5 --timeout 15000
   ```

2. Pick relevant URLs from `results[].url`.

3. Fetch each page you need:

   ```bash
   fetch-content "https://relevant-url"
   ```

4. Use `search-web --fetch --fetch-limit 5` for an all-in-one call (search + fetch top results).

5. **Reddit URLs**: `www.reddit.com` is automatically rewritten to `old.reddit.com` (no manual rewrite needed):

   ```bash
   fetch-content "https://www.reddit.com/r/.../comments/.../"
   ```

## Search Engines

- **Brave (default)**: Fast, no login required, Obscura-based. Region: all, Language: English.
- **Google (with `--google`)**: Requires browser profile, supports human-in-the-loop CAPTCHA solving.

## Rate Limiting

Brave searches are automatically spaced 10 seconds apart to avoid triggering bot detection. Timestamp stored in `/tmp/search-headless-last-search` (RAM). No configuration needed.

## Output Formats

Both commands support `--format json` (default) and `--format markdown`:

```bash
search-web "query" --format markdown
fetch-content "url" --format markdown
```

## Status Handling

| Status | Meaning | Agent action |
|---|---|---|
| `ok` | Command succeeded | Use results/content |
| `partial` | Search worked but some fetches failed | Use successful entries, inspect per-result errors |
| `blocked` | Search engine blocked | Retry later or inspect `errors[]` |
| `timeout` | Timeout expired | Retry with larger `--timeout` or narrower query |
| `error` | Unrecoverable failure | Read `errors[]` |

Exit code is `0` for `ok` and `partial`; non-zero for `blocked`, `timeout`, and `error`.
