# Search Headless Design Spec

Date: 2026-04-26

## Overview

Build a Bun + TypeScript CLI toolset intended for LLM coding agents. The tools provide web search through Google in a real browser session and readable content extraction from result pages or arbitrary URLs.

The primary commands are:

```bash
search-web "query"
search-web "query" --fetch --fetch-limit 5
fetch-content "https://example.com/article"
```

The default output format is machine-readable JSON on stdout, with optional Markdown output. Progress and human-readable warnings go to stderr.

The system uses normal browser automation and human-in-the-loop handling for CAPTCHA, consent, or anti-bot pages. It does not implement CAPTCHA solving, stealth plugins, browser fingerprint spoofing, proxy rotation, or automatic anti-bot circumvention.

## Goals

- Give LLM agents simple bash-callable commands for search and page content retrieval.
- Avoid paid search APIs by using a local Playwright Chromium browser for Google searches.
- Return stable JSON contracts by default.
- Support optional Markdown output for direct LLM consumption.
- Preserve browser session state across calls with a persistent profile.
- Detect CAPTCHA/block pages and pause for manual human intervention when allowed.
- Fetch page content efficiently with HTTP-first extraction and browser fallback.

## Non-goals

- No CAPTCHA bypass automation.
- No stealth or anti-detection plugin usage.
- No proxy rotation or abuse-oriented evasion.
- No guarantee that Google will always permit automated access.
- No long-running daemon in the initial version.
- No scraping of localhost/private network URLs by default.

## Core Concepts

### CLI Command

Purpose: Provide a stable interface an AI agent can invoke via bash.

Operational principle: The agent invokes `search-web` or `fetch-content` with flags; the command writes structured output to stdout and diagnostics to stderr.

State: Command args, output format, timeout, execution result.

### Browser Session

Purpose: Search Google through a real browser context while preserving cookies and consent/session state.

Operational principle: Playwright launches Chromium using a persistent profile directory, navigates to Google, waits for results or block signals, and parses result entries.

State: Profile directory, cookies, storage, lock file.

### Human Intervention

Purpose: Let a human solve consent/CAPTCHA/unusual-traffic challenges manually without implementing bypass logic.

Operational principle: If a challenge is detected and human intervention is enabled, the tool opens or keeps a visible browser window, prints instructions to stderr, waits for results to appear, then resumes.

State: intervention allowed, timeout, challenge detection status.

### Search Result

Purpose: Represent one Google organic result in an agent-friendly way.

Operational principle: Extract rank, title, URL, display URL, and snippet from the rendered search results page.

State: rank, title, URL, display URL, snippet, optional fetched content.

### Content Extraction

Purpose: Turn a URL into readable article/page text.

Operational principle: Try HTTP fetch and Readability extraction first; fall back to browser rendering when HTTP extraction fails or produces low-quality content.

State: URL, method, title, metadata, text, truncation state, errors.

## Architecture

Project layout:

```text
search-headless/
  package.json
  tsconfig.json
  src/
    cli/
      search-web.ts
      fetch-content.ts
    search/
      google.ts
      parse-results.ts
      block-detection.ts
    fetch/
      fetch-content.ts
      http-extract.ts
      browser-extract.ts
    browser/
      context.ts
      human-intervention.ts
    output/
      json.ts
      markdown.ts
    types.ts
```

### CLI Layer

Responsibilities:

- Parse command arguments.
- Apply defaults.
- Call search or fetch modules.
- Render JSON or Markdown.
- Ensure stdout remains parseable.
- Send progress/warnings to stderr.
- Set appropriate exit codes.

### Browser Search Layer

Responsibilities:

- Launch Chromium with a persistent profile directory.
- Use Google Search for query discovery.
- Detect consent, CAPTCHA, unusual traffic, and blocked pages.
- Support human-in-the-loop intervention.
- Parse organic search results.
- Paginate cautiously when `--limit` exceeds one page.

### Content Extraction Layer

Responsibilities:

- Validate URLs and block local/private URLs by default.
- Fetch HTML through HTTP.
- Parse readable content with `jsdom` and `@mozilla/readability`.
- Fall back to Playwright rendering for JS-heavy or failed pages.
- Return structured extraction metadata.

### Output Layer

Responsibilities:

- Define stable JSON result schemas.
- Provide Markdown rendering for search results and fetched content.
- Truncate long content consistently.

## CLI Contract

### `search-web`

```bash
search-web "<query>" [options]
```

Options:

```text
--limit <n>              Number of search results to return. Default: 10.
--fetch                  Fetch content for top results.
--fetch-limit <n>        Number of result URLs to fetch when --fetch is set. Default: 5.
--format <json|markdown> Output format. Default: json.
--headful                Force visible browser.
--no-human               Do not wait for human CAPTCHA/consent intervention; fail fast.
--timeout <ms>           Overall timeout. Default: 60000.
--region <code>          Optional Google region/language tuning later.
```

Default behavior:

- Return up to 10 search results.
- Do not fetch result content unless `--fetch` is set.
- With `--fetch`, fetch content for up to 5 top results by default.

Example JSON output without fetch:

```json
{
  "query": "example query",
  "engine": "google",
  "status": "ok",
  "results": [
    {
      "rank": 1,
      "title": "Result title",
      "url": "https://example.com",
      "displayUrl": "example.com",
      "snippet": "Short Google snippet..."
    }
  ],
  "errors": []
}
```

Example JSON output with fetch:

```json
{
  "query": "example query",
  "engine": "google",
  "status": "ok",
  "results": [
    {
      "rank": 1,
      "title": "Result title",
      "url": "https://example.com",
      "displayUrl": "example.com",
      "snippet": "Short Google snippet...",
      "fetched": {
        "status": "ok",
        "title": "Page title",
        "url": "https://example.com",
        "text": "Extracted readable text...",
        "excerpt": "Short summary/excerpt...",
        "method": "http",
        "truncated": false,
        "textLength": 12345,
        "errors": []
      }
    }
  ],
  "errors": []
}
```

### `fetch-content`

```bash
fetch-content "<url>" [options]
```

Options:

```text
--format <json|markdown> Output format. Default: json.
--browser                Force browser extraction.
--no-browser             Disable browser fallback.
--headful                Force visible browser.
--no-human               Fail fast if human intervention would be needed.
--timeout <ms>           Overall timeout. Default: 60000.
--max-chars <n>          Truncate returned text. Default: 50000.
```

Example JSON output:

```json
{
  "url": "https://example.com/article",
  "status": "ok",
  "title": "Article title",
  "siteName": "Example",
  "byline": "Author",
  "publishedTime": "2026-04-26",
  "excerpt": "Short excerpt...",
  "text": "Main extracted content...",
  "method": "http",
  "truncated": false,
  "textLength": 12345,
  "errors": []
}
```

## Browser Behavior

The browser profile directory defaults to:

```text
~/.search-headless/browser-profile/
```

This preserves Google consent state, cookies, session data, language/location preferences, and any manually completed challenge state where applicable.

Normal search flow:

1. Launch Chromium with persistent profile.
2. Navigate to Google Search with the query.
3. Wait for search result elements.
4. Parse organic result entries.
5. If fewer than requested results are found and `--limit` exceeds one page, paginate cautiously.
6. Return structured results.

## CAPTCHA, Consent, and Block Handling

Detection signals include:

- Google `/sorry/` URLs.
- Text such as “unusual traffic”.
- Text such as “Our systems have detected unusual traffic”.
- CAPTCHA iframe/challenge indicators.
- Missing expected result containers while challenge text is present.

Default behavior when human intervention is allowed:

1. Print a clear message to stderr.
2. Open or keep the browser visible.
3. Ask the human to solve the challenge manually.
4. Wait until search results become visible, timeout expires, or the user aborts.
5. Continue parsing results after the challenge is solved.

Example stderr message:

```text
Human intervention required:
Google appears to be showing a CAPTCHA or unusual traffic page.
Please solve it in the opened browser window. The command will continue automatically.
Timeout: 120 seconds.
```

With `--no-human`, fail fast with structured output:

```json
{
  "query": "example",
  "engine": "google",
  "status": "blocked",
  "results": [],
  "errors": [
    {
      "code": "HUMAN_INTERVENTION_REQUIRED",
      "message": "Google showed a CAPTCHA or unusual traffic page.",
      "recoverable": true
    }
  ]
}
```

## Content Fetching and Extraction

Extraction flow:

1. Validate URL.
2. Reject non-HTTP(S) URLs.
3. Reject local/private URLs by default.
4. Try HTTP fetch and Readability extraction.
5. Return HTTP extraction if quality is sufficient.
6. Fall back to browser extraction if needed and allowed.
7. Truncate text according to `--max-chars`.

Private URL blocking includes, where detectable:

- `localhost`
- `127.0.0.0/8`
- `10.0.0.0/8`
- `172.16.0.0/12`
- `192.168.0.0/16`
- link-local/private hostnames

Browser fallback triggers:

- HTTP request failure.
- Non-HTML response.
- Extracted text below minimum useful length.
- JS-heavy page indicators.
- Blocked or paywalled page indicators.
- Readability failure.

Browser extraction behavior:

- Open URL in Playwright.
- Wait for DOM content loaded/network idle within timeout.
- Remove obvious noise elements: scripts, styles, nav, footer, aside, common cookie banners.
- Extract rendered readable text.
- Return `method: "browser"`.

For `search-web --fetch`, per-result fetch failures do not fail the entire command. The result receives a `fetched.status = "error"` object. The top-level status becomes `partial` if at least one fetch failed while search succeeded.

## Error Handling

Top-level statuses:

```text
ok          Command succeeded.
partial     Search succeeded, but some fetches failed.
blocked     Google/page requires human intervention and --no-human was used or timeout expired.
timeout     Overall timeout expired.
error       Unexpected unrecoverable error.
```

Standard error object:

```json
{
  "code": "FETCH_TIMEOUT",
  "message": "Timed out after 15000ms.",
  "recoverable": true,
  "details": {}
}
```

Initial error codes:

```text
GOOGLE_BLOCKED
HUMAN_INTERVENTION_REQUIRED
HUMAN_INTERVENTION_TIMEOUT
SEARCH_PARSE_FAILED
FETCH_TIMEOUT
FETCH_FORBIDDEN
FETCH_NOT_HTML
READABILITY_FAILED
BROWSER_NAVIGATION_FAILED
INVALID_URL
PRIVATE_URL_BLOCKED
BROWSER_PROFILE_LOCKED
UNKNOWN_ERROR
```

Exit codes:

- `0` for `ok` and `partial`.
- Non-zero for `blocked`, `timeout`, or `error`.

## Concurrency and Profile Locking

Use a lock file to prevent concurrent persistent-profile access:

```text
~/.search-headless/lock
```

If another command is running, wait briefly by default. If the lock cannot be acquired within the configured timeout, fail with `BROWSER_PROFILE_LOCKED`.

## Configuration

Initial implementation should prioritize CLI flags. Future config file support can use:

```text
~/.search-headless/config.json
```

Initial defaults:

```json
{
  "searchLimit": 10,
  "fetchLimit": 5,
  "timeoutMs": 60000,
  "maxChars": 50000,
  "humanIntervention": true,
  "profileDir": "~/.search-headless/browser-profile"
}
```

## Testing Strategy

### Unit Tests

- URL validation and private URL blocking.
- JSON output formatting.
- Markdown output formatting.
- Google result parser against saved fixture HTML.
- Block/CAPTCHA detection against saved fixture HTML.
- Readability extraction against saved fixture pages.

### Integration Tests

- `fetch-content` against a local test HTTP server.
- Browser fallback against a local JS-rendered page.
- CLI output and exit codes.
- Lock handling.

### Manual Smoke Tests

- Real Google search.
- `search-web "query" --fetch`.
- Manual CAPTCHA/consent flow if triggered.
- Markdown output.

## Acceptance Criteria

- `search-web "query"` returns JSON with up to 10 Google results by default.
- `search-web "query" --limit 20` attempts to return up to 20 results.
- `search-web "query" --fetch` fetches content for up to 5 top results by default.
- `fetch-content "https://example.com"` returns extracted text using HTTP when possible.
- Browser fallback works for a JS-rendered local test page.
- CAPTCHA/block detection produces structured errors with `--no-human`.
- Human intervention mode opens or keeps a visible browser and resumes after manual completion.
- stdout remains parseable JSON by default.
- diagnostics and progress messages go to stderr.
- private/local URLs are rejected by default.
- concurrent profile usage is guarded by a lock.
