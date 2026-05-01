---
name: search-fetch
description: Use when the agent needs up-to-date knowledge beyond its training cutoff, when the user provides a URL to fetch, or when the agent must search the web to answer a question.
---

# Search & Fetch

Two CLI commands: `search-web` for Google search, `fetch-content` for page extraction (uses obscura headless browser internally). JSON on stdout, diagnostics on stderr.

## Commands

### search-web — Search Google

```bash
search-web "query" --limit 5 --timeout 15000
```

Returns JSON:

```json
{
  "query": "query",
  "engine": "google",
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
--headful                Force visible browser (for CAPTCHA solving)
--no-human               Fail fast if Google blocks
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

5. **Reddit URLs**: `www.reddit.com` blocks headless browsers. Always rewrite to `old.reddit.com` before fetching:

   ```bash
   fetch-content "https://old.reddit.com/r/.../comments/.../"
   ```

## Status Handling

| Status | Meaning | Agent action |
|---|---|---|
| `ok` | Command succeeded | Use results/content |
| `partial` | Search worked but some fetches failed | Use successful entries, inspect per-result errors |
| `blocked` | Google/page blocked | Retry later or inspect `errors[]` |
| `timeout` | Timeout expired | Retry with larger `--timeout` or narrower query |
| `error` | Unrecoverable failure | Read `errors[]` |

Exit code is `0` for `ok` and `partial`; non-zero for `blocked`, `timeout`, and `error`.
