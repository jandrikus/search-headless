---
name: search-web
description: Use when an AI coding agent needs to search Google from the shell or fetch readable content from a URL using the local search-web and fetch-content commands.
---

# Search Web CLI Skill

## Overview

Use the local `search-web` and `fetch-content` shell commands to discover web pages and extract readable page content. These tools are designed for AI coding agents: JSON is written to stdout by default, diagnostics go to stderr, and failures are structured.

## Commands

### Search Google

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

### Search and fetch top results

```bash
search-web "query" --limit 10 --fetch --fetch-limit 5 --timeout 30000
```

Use this when you want result metadata plus readable content from the top URLs.

### Fetch one URL

```bash
fetch-content "https://example.com/article" --max-chars 50000
```

Returns extracted readable text and metadata.

### Markdown output

```bash
search-web "query" --format markdown
fetch-content "https://example.com" --format markdown
```

Use Markdown when the next step is direct reading by an LLM rather than strict JSON parsing.

## Recommended Agent Workflow

1. Start with search metadata:

   ```bash
   search-web "specific query" --limit 5 --timeout 15000
   ```

2. Pick relevant URLs from `results[].url`.

3. Fetch only the pages you need:

   ```bash
   fetch-content "https://relevant-url" --max-chars 50000
   ```

4. Use `search-web --fetch --fetch-limit 5` only when you need an all-in-one call.

## Status Handling

| Status | Meaning | Agent action |
|---|---|---|
| `ok` | Command succeeded | Use results/content |
| `partial` | Search worked but some fetches failed | Use successful entries, inspect per-result errors |
| `blocked` | Google/page needs human intervention | Retry later or ask human to run without `--no-human` |
| `timeout` | Timeout expired | Retry with larger `--timeout` or narrower query |
| `error` | Unrecoverable or unexpected failure | Read `errors[]` |

Exit code is `0` for `ok` and `partial`; non-zero for `blocked`, `timeout`, and `error`.

## Human Intervention

By default, `search-web` may open a visible browser if Google shows consent, CAPTCHA, or unusual-traffic pages. A human can solve the prompt, and the command will continue.

For fully non-interactive agent runs, use:

```bash
search-web "query" --no-human --timeout 15000
```

If this returns `status: "blocked"`, do not try to bypass it. Ask for human intervention or retry later.

## Safety Rules

- Do not automate CAPTCHA solving.
- Do not add stealth plugins, fingerprint spoofing, or proxy rotation.
- Do not fetch localhost or private network URLs; the tool blocks them by default.
- Keep stdout machine-readable. Do not mix progress logs into stdout.

