# Search Headless Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Bun + TypeScript CLI toolset with `search-web` and `fetch-content` commands for agent-friendly Google search and page content extraction.

**Architecture:** The project is a small CLI package split into focused modules: command entrypoints, shared types/config, browser/session handling, Google search parsing, content extraction, and output formatting. Search uses Playwright Chromium with a persistent local profile and human-in-the-loop challenge handling; content fetching uses HTTP + Readability first, with Playwright browser fallback.

**Tech Stack:** Bun, TypeScript, Playwright, Commander, jsdom, @mozilla/readability, Cheerio, Vitest.

---

## File Structure

Create these files:

```text
package.json                         Package metadata, bin mappings, scripts, dependencies.
tsconfig.json                        TypeScript configuration for Bun.
.gitignore                           Ignore deps, build outputs, browser profile artifacts.
README.md                            Usage, safety boundary, command examples.
src/types.ts                         Shared result, error, config, and option types.
src/config.ts                        Defaults and config helpers.
src/cli/search-web.ts                `search-web` executable entrypoint.
src/cli/fetch-content.ts             `fetch-content` executable entrypoint.
src/output/json.ts                   JSON serialization helpers.
src/output/markdown.ts               Markdown rendering helpers.
src/browser/context.ts               Playwright profile, launch, and lock handling.
src/browser/human-intervention.ts    Human challenge wait/resume behavior.
src/search/block-detection.ts        CAPTCHA/consent/block signal detection.
src/search/parse-results.ts          Google organic result parsing.
src/search/google.ts                 Google search orchestration.
src/fetch/url-validation.ts          HTTP(S) URL and private-address validation.
src/fetch/http-extract.ts            HTTP fetch + Readability extraction.
src/fetch/browser-extract.ts         Browser-rendered content extraction.
src/fetch/fetch-content.ts           Hybrid extraction orchestration.
tests/fixtures/google-results.html   Stable Google-like fixture.
tests/fixtures/google-blocked.html   Block/CAPTCHA-like fixture.
tests/fixtures/article.html          Readability fixture.
tests/unit/*.test.ts                 Unit tests.
tests/integration/*.test.ts          CLI/local server/browser fallback tests.
```

---

## Global Implementation Constraints From Plan Review

Apply these constraints throughout implementation:

1. **Private URL blocking remains default.** Internal tests that need local servers must use an internal-only option `allowPrivateUrls: true` or the test-only environment variable `SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS=1`. The normal CLI default must reject localhost/private URLs with structured `PRIVATE_URL_BLOCKED` JSON and non-zero exit.
2. **Redirect and DNS safety are required.** URL fetching must re-validate every redirect target before following it, reject final private URLs, reject `0.0.0.0`, `169.254.169.254`, IPv6 loopback/private/link-local ranges, and use an injectable DNS resolver for tests so private DNS resolution can be detected where possible.
3. **Overall timeout is a deadline, not per-step budget.** Convert `--timeout` into a shared deadline: `deadline = Date.now() + timeoutMs`; each phase receives `remainingMs(deadline)`.
4. **Human intervention requires a visible browser.** Either launch headful whenever human intervention is enabled, or relaunch headful when a challenge is detected. The implementation must test that intervention uses a visible/headful path.
5. **Consent pages count as recoverable blocks.** Detect `consent.google.com`, “Before you continue to Google”, consent forms/buttons, and return/invoke `HUMAN_INTERVENTION_REQUIRED` consistently.
6. **Pagination is required for `--limit > 10`.** Search must cautiously fetch/click next pages until limit reached, no next page exists, timeout/deadline expires, or a block/challenge occurs. Results must be deduplicated and globally ranked.
7. **Git commits are optional.** Only commit if the user explicitly wants commits and the directory is a git repo. Never use `git add .`; review `git status --short` and stage only intentional files. Do not hide commit failures with `|| true`.
8. **`--region` is deferred for v0.1.** Do not expose it in the first implementation, or expose it only as a documented no-op with a test. Prefer not exposing it until implemented.
9. **Standalone `fetch-content` statuses:** `ok` when content extracted cleanly; `partial` when content extracted after a recoverable prior failure; `timeout` for deadline expiry; `blocked` for human intervention required/disabled or timed out; `error` when no extraction method succeeds.

---

## Chunk 1: Project Scaffold and Shared Contracts

### Task 1: Initialize Bun TypeScript package

**Files:**
- Create: `package.json`
- Create: `tsconfig.json`
- Create: `.gitignore`

- [ ] **Step 1: Create package metadata**

Create `package.json`:

```json
{
  "name": "search-headless",
  "version": "0.1.0",
  "type": "module",
  "private": true,
  "bin": {
    "search-web": "./src/cli/search-web.ts",
    "fetch-content": "./src/cli/fetch-content.ts"
  },
  "scripts": {
    "test": "vitest run",
    "test:watch": "vitest",
    "typecheck": "tsc --noEmit",
    "smoke:fetch": "bun run src/cli/fetch-content.ts https://example.com --no-browser",
    "smoke:search": "bun run src/cli/search-web.ts \"bun typescript\" --limit 3 --no-human"
  },
  "dependencies": {
    "@mozilla/readability": "latest",
    "cheerio": "latest",
    "commander": "latest",
    "jsdom": "latest",
    "playwright": "latest"
  },
  "devDependencies": {
    "@types/bun": "latest",
    "@types/jsdom": "latest",
    "typescript": "latest",
    "vitest": "latest"
  }
}
```

- [ ] **Step 2: Create TypeScript config**

Create `tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "types": ["bun-types"],
    "noEmit": true
  },
  "include": ["src/**/*.ts", "tests/**/*.ts"]
}
```

- [ ] **Step 3: Create ignore file**

Create `.gitignore`:

```gitignore
node_modules/
coverage/
dist/
.search-headless/
*.log
.DS_Store
```

- [ ] **Step 4: Install dependencies**

Run:

```bash
bun install
```

Expected: lockfile created and dependencies installed.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "chore: scaffold bun typescript cli package"
```

Do not use `git add .`; do not suppress git failures.

If this directory is not a git repo, skip commit and note it.

### Task 2: Define shared types and defaults with tests

**Files:**
- Create: `src/types.ts`
- Create: `src/config.ts`
- Create: `tests/unit/config.test.ts`

- [ ] **Step 1: Write failing config test**

Create `tests/unit/config.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { DEFAULT_CONFIG } from "../../src/config";

it("defines agent-friendly defaults", () => {
  expect(DEFAULT_CONFIG.searchLimit).toBe(10);
  expect(DEFAULT_CONFIG.fetchLimit).toBe(5);
  expect(DEFAULT_CONFIG.timeoutMs).toBe(60_000);
  expect(DEFAULT_CONFIG.maxChars).toBe(50_000);
  expect(DEFAULT_CONFIG.humanIntervention).toBe(true);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
bun test tests/unit/config.test.ts
```

Expected: FAIL because `src/config.ts` does not exist.

- [ ] **Step 3: Implement types and defaults**

Create `src/types.ts` with exported union types and interfaces:

```ts
export type OutputFormat = "json" | "markdown";
export type CommandStatus = "ok" | "partial" | "blocked" | "timeout" | "error";
export type FetchMethod = "http" | "browser";

export interface ToolError {
  code: string;
  message: string;
  recoverable: boolean;
  details?: Record<string, unknown>;
}

export interface AppConfig {
  searchLimit: number;
  fetchLimit: number;
  timeoutMs: number;
  maxChars: number;
  humanIntervention: boolean;
  profileDir: string;
  lockFile: string;
}

export interface Deadline {
  expiresAt: number;
}

export function remainingMs(deadline: Deadline): number {
  return Math.max(0, deadline.expiresAt - Date.now());
}

export interface FetchedContent {
  url: string;
  status: CommandStatus;
  title?: string;
  siteName?: string;
  byline?: string;
  publishedTime?: string;
  excerpt?: string;
  text?: string;
  method?: FetchMethod;
  truncated?: boolean;
  textLength?: number;
  errors: ToolError[];
}

export interface SearchResult {
  rank: number;
  title: string;
  url: string;
  displayUrl?: string;
  snippet?: string;
  fetched?: FetchedContent;
}

export interface SearchResponse {
  query: string;
  engine: "google";
  status: CommandStatus;
  results: SearchResult[];
  errors: ToolError[];
}
```

Create `src/config.ts`:

```ts
import { homedir } from "node:os";
import { join } from "node:path";
import type { AppConfig } from "./types";

const baseDir = join(homedir(), ".search-headless");

export const DEFAULT_CONFIG: AppConfig = {
  searchLimit: 10,
  fetchLimit: 5,
  timeoutMs: 60_000,
  maxChars: 50_000,
  humanIntervention: true,
  profileDir: join(baseDir, "browser-profile"),
  lockFile: join(baseDir, "lock"),
};
```

- [ ] **Step 4: Run test to verify it passes**

Run:

```bash
bun test tests/unit/config.test.ts
```

Expected: PASS.

- [ ] **Step 5: Run typecheck**

Run:

```bash
bun run typecheck
```

Expected: exit 0.

- [ ] **Step 6: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: add shared result contracts and defaults"
```

Do not use `git add .`; do not suppress git failures.

---

## Chunk 2: Output Formatters and URL Validation

### Task 3: Implement JSON and Markdown output

**Files:**
- Create: `src/output/json.ts`
- Create: `src/output/markdown.ts`
- Create: `tests/unit/output.test.ts`

- [ ] **Step 1: Write failing output tests**

Create `tests/unit/output.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { renderJson } from "../../src/output/json";
import { renderSearchMarkdown } from "../../src/output/markdown";
import type { SearchResponse } from "../../src/types";

const response: SearchResponse = {
  query: "example",
  engine: "google",
  status: "ok",
  results: [
    { rank: 1, title: "Example", url: "https://example.com", displayUrl: "example.com", snippet: "Demo" },
  ],
  errors: [],
};

it("renders pretty JSON", () => {
  expect(JSON.parse(renderJson(response))).toEqual(response);
});

it("renders search markdown", () => {
  const markdown = renderSearchMarkdown(response);
  expect(markdown).toContain("# Search results for `example`");
  expect(markdown).toContain("1. [Example](https://example.com)");
  expect(markdown).toContain("Demo");
});
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
bun test tests/unit/output.test.ts
```

Expected: FAIL because output modules do not exist.

- [ ] **Step 3: Implement output modules**

Create `src/output/json.ts`:

```ts
export function renderJson(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}
```

Create `src/output/markdown.ts`:

```ts
import type { FetchedContent, SearchResponse } from "../types";

export function renderSearchMarkdown(response: SearchResponse): string {
  const lines = [`# Search results for \`${response.query}\``, ""];
  for (const result of response.results) {
    lines.push(`${result.rank}. [${result.title}](${result.url})`);
    if (result.displayUrl) lines.push(`   - URL: ${result.displayUrl}`);
    if (result.snippet) lines.push(`   - Snippet: ${result.snippet}`);
    if (result.fetched?.text) {
      lines.push("", `   ## Fetched content`, "", indent(result.fetched.text, "   "));
    }
    lines.push("");
  }
  if (response.errors.length > 0) {
    lines.push("## Errors", "");
    for (const error of response.errors) lines.push(`- ${error.code}: ${error.message}`);
  }
  return `${lines.join("\n").trim()}\n`;
}

export function renderFetchedMarkdown(content: FetchedContent): string {
  const lines = [`# ${content.title ?? content.url}`, "", `URL: ${content.url}`, ""];
  if (content.excerpt) lines.push(`> ${content.excerpt}`, "");
  if (content.text) lines.push(content.text, "");
  if (content.errors.length > 0) {
    lines.push("## Errors", "");
    for (const error of content.errors) lines.push(`- ${error.code}: ${error.message}`);
  }
  return `${lines.join("\n").trim()}\n`;
}

function indent(text: string, prefix: string): string {
  return text.split("\n").map((line) => `${prefix}${line}`).join("\n");
}
```

- [ ] **Step 4: Run tests**

Run:

```bash
bun test tests/unit/output.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: add json and markdown renderers"
```

Do not use `git add .`; do not suppress git failures.

### Task 4: Implement URL validation and private URL blocking

**Files:**
- Create: `src/fetch/url-validation.ts`
- Create: `tests/unit/url-validation.test.ts`

- [ ] **Step 1: Write failing URL validation tests**

Create `tests/unit/url-validation.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { validatePublicHttpUrl } from "../../src/fetch/url-validation";

it("accepts public http and https urls", () => {
  expect(validatePublicHttpUrl("https://example.com/a").ok).toBe(true);
  expect(validatePublicHttpUrl("http://example.com/a").ok).toBe(true);
});

it("rejects non-http urls", () => {
  const result = validatePublicHttpUrl("file:///etc/passwd");
  expect(result.ok).toBe(false);
  expect(result.error?.code).toBe("INVALID_URL");
});

it("rejects local, private, metadata, and IPv6 local hosts", () => {
  for (const url of [
    "http://localhost",
    "http://127.0.0.1",
    "http://0.0.0.0",
    "http://10.0.0.1",
    "http://172.16.0.1",
    "http://192.168.1.1",
    "http://169.254.169.254",
    "http://[::1]",
    "http://[fc00::1]",
    "http://[fd00::1]",
    "http://[fe80::1]",
    "http://[::ffff:127.0.0.1]",
    "http://[::ffff:10.0.0.1]",
  ]) {
    const result = validatePublicHttpUrl(url);
    expect(result.ok, url).toBe(false);
    expect(result.error?.code).toBe("PRIVATE_URL_BLOCKED");
  }
});

it("can allow private urls only through an internal test option", () => {
  expect(validatePublicHttpUrl("http://127.0.0.1", { allowPrivateUrls: true }).ok).toBe(true);
});
```

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
bun test tests/unit/url-validation.test.ts
```

Expected: FAIL because module does not exist.

- [ ] **Step 3: Implement validation**

Create `src/fetch/url-validation.ts`:

```ts
import type { ToolError } from "../types";

export type UrlValidation = { ok: true; url: URL } | { ok: false; error: ToolError };

export interface UrlValidationOptions {
  allowPrivateUrls?: boolean;
}

export function validatePublicHttpUrl(input: string, options: UrlValidationOptions = {}): UrlValidation {
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    return fail("INVALID_URL", "URL is not valid.");
  }

  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return fail("INVALID_URL", "Only http and https URLs are supported.");
  }

  const host = url.hostname.toLowerCase();
  if (!options.allowPrivateUrls && isPrivateHost(host)) {
    return fail("PRIVATE_URL_BLOCKED", "Local and private network URLs are blocked by default.");
  }

  return { ok: true, url };
}

function fail(code: string, message: string): UrlValidation {
  return { ok: false, error: { code, message, recoverable: false } };
}

function isPrivateHost(host: string): boolean {
  const normalized = host.toLowerCase().replace(/^\[/, "").replace(/\]$/, "");
  if (normalized === "localhost" || normalized.endsWith(".localhost")) return true;

  const mappedIpv4 = normalized.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/)?.[1];
  if (mappedIpv4) return isPrivateIpv4(mappedIpv4);

  if (normalized === "::1") return true;
  if (normalized.startsWith("fc") || normalized.startsWith("fd")) return true; // fc00::/7 unique-local
  if (/^fe[89ab]:/i.test(normalized)) return true; // fe80::/10 link-local, coarse textual check

  return isPrivateIpv4(normalized);
}

function isPrivateIpv4(host: string): boolean {
  const parts = host.split(".").map(Number);
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) return false;
  const [a, b] = parts;
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 169 && b === 254) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168)
  );
}
```

- [ ] **Step 4: Run tests**

Run:

```bash
bun test tests/unit/url-validation.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: block invalid and private fetch urls"
```

Do not use `git add .`; do not suppress git failures.

---

## Chunk 3: HTTP Content Extraction and CLI

### Task 5: Implement HTTP Readability extraction and redirect safety

**Files:**
- Create: `src/fetch/http-extract.ts`
- Create: `src/fetch/fetch-content.ts`
- Create: `tests/fixtures/article.html`
- Create: `tests/unit/http-extract.test.ts`

- [ ] **Step 1: Create article fixture and failing test**

Create `tests/fixtures/article.html`:

```html
<!doctype html>
<html>
  <head>
    <title>Fixture Article</title>
    <meta name="description" content="Fixture excerpt">
    <meta property="og:site_name" content="Fixture Site">
    <meta name="author" content="Test Author">
    <meta property="article:published_time" content="2026-04-26">
  </head>
  <body>
    <article>
      <h1>Fixture Article</h1>
      <p>This is a useful paragraph with enough text to be extracted by readability.</p>
      <p>This is another useful paragraph with meaningful article content for the parser.</p>
    </article>
  </body>
</html>
```

Create `tests/unit/http-extract.test.ts` with a local Bun server serving the fixture through internal options `{ allowPrivateUrls: true }` and asserting title/text/method/siteName/byline/publishedTime/excerpt. Add tests for truncation (`maxChars: 20`), redirect to localhost rejection by default, and mocked DNS resolver returning a private IP.

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/unit/http-extract.test.ts
```

Expected: FAIL because extraction modules do not exist.

- [ ] **Step 3: Implement HTTP extraction**

Implement `extractViaHttp(url, options)` in `src/fetch/http-extract.ts`:

- Use `fetch` with abort timeout based on `remainingMs(deadline)`.
- Use manual redirect handling so each redirect target is validated before following.
- Re-check the final response URL before parsing.
- Accept an injectable DNS resolver for tests and reject hostnames resolving to private/link-local/reserved addresses where detectable.
- Require `content-type` containing `text/html` or empty content-type.
- Parse with `JSDOM`.
- Use `new Readability(dom.window.document).parse()`.
- Extract metadata from Readability plus common meta tags: `og:site_name`, `author`, `article:published_time`, and `description`.
- Return `FetchedContent` with `method: "http"`.
- Truncate text to `maxChars` and set `truncated`/`textLength`; `textLength` must be the original extracted length before truncation.
- Return structured error objects for timeout, redirect-to-private, DNS-private, non-HTML, forbidden, and readability failure.

- [ ] **Step 4: Implement orchestration shell**

Create `src/fetch/fetch-content.ts` with `fetchContent(url, options)` that validates URL and calls HTTP extraction. Browser fallback will be added in a later task.

- [ ] **Step 5: Run HTTP extraction tests**

Run:

```bash
bun test tests/unit/http-extract.test.ts
```

Expected: PASS.

- [ ] **Step 6: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: extract readable content over http"
```

Do not use `git add .`; do not suppress git failures.

### Task 6: Implement `fetch-content` CLI

**Files:**
- Create: `src/cli/fetch-content.ts`
- Create: `tests/integration/fetch-content-cli.test.ts`

- [ ] **Step 1: Write failing CLI integration test**

Create tests that start a local server and run the CLI in test mode:

```bash
SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS=1 bun run src/cli/fetch-content.ts <local-url> --no-browser
```

Assert:

- exit code 0
- stdout parses as JSON
- `status` is `ok`
- `text` contains fixture text
- stderr is allowed but not required

Also add default safety tests without the env var:

```bash
bun run src/cli/fetch-content.ts http://localhost:1234 --no-browser
```

Assert:

- non-zero exit
- stdout parses as JSON
- `status` is `error`
- first error code is `PRIVATE_URL_BLOCKED`
- diagnostics, if any, are on stderr only

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/integration/fetch-content-cli.test.ts
```

Expected: FAIL because CLI entrypoint does not exist.

- [ ] **Step 3: Implement CLI**

Create `src/cli/fetch-content.ts`:

- Add shebang `#!/usr/bin/env bun`.
- Use Commander.
- Args: `<url>`.
- Options: `--format`, `--browser`, `--no-browser`, `--headful`, `--no-human`, `--timeout`, `--max-chars`.
- Do not expose an allow-private flag. Only honor `SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS=1` in test runs.
- Convert timeout to a shared deadline and pass remaining time to all phases.
- Call `fetchContent`.
- Write JSON or Markdown to stdout.
- Write diagnostics to stderr only.
- Exit 0 for `ok`/`partial`, non-zero otherwise.
- Add tests for invalid URL, private URL, invalid format, Markdown output, `--browser`, `--no-browser`, non-HTML fallback decisions, and status rules (`ok`, `partial`, `timeout`, `blocked`, `error`).

- [ ] **Step 4: Run CLI test**

Run:

```bash
bun test tests/integration/fetch-content-cli.test.ts
```

Expected: PASS.

- [ ] **Step 5: Run typecheck and full current test suite**

Run:

```bash
bun run typecheck
bun test
```

Expected: both exit 0.

- [ ] **Step 6: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: add fetch-content cli"
```

Do not use `git add .`; do not suppress git failures.

---

## Chunk 4: Browser Context, Fallback Extraction, and Human Intervention

### Task 7: Implement browser context and profile locking

**Files:**
- Create: `src/browser/context.ts`
- Create: `tests/unit/browser-lock.test.ts`

- [ ] **Step 1: Write failing lock tests**

Test that acquiring a lock creates a lock file and a second immediate acquisition fails or times out with `BROWSER_PROFILE_LOCKED`.

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/unit/browser-lock.test.ts
```

Expected: FAIL.

- [ ] **Step 3: Implement lock and browser helpers**

Create `src/browser/context.ts`:

- `withProfileLock(lockFile, deadline, fn)`.
- Atomic lock creation using exclusive file creation where possible.
- Remove lock in `finally`.
- `launchPersistentBrowser(options)` using `chromium.launchPersistentContext(profileDir, { headless: !(options.headful || options.humanIntervention) })` for flows where human intervention may be required.
- If the implementation chooses headless-first search, it must close and relaunch headful when a challenge is detected and human intervention is allowed.

- [ ] **Step 4: Run lock tests**

Run:

```bash
bun test tests/unit/browser-lock.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: add browser profile locking"
```

Do not use `git add .`; do not suppress git failures.

### Task 8: Implement browser fallback extraction

**Files:**
- Create: `src/fetch/browser-extract.ts`
- Modify: `src/fetch/fetch-content.ts`
- Create: `tests/integration/browser-extract.test.ts`

- [ ] **Step 1: Write failing browser fallback test**

Create a local HTML page that inserts article text via JavaScript. Call internal API with `{ browserFallback: true, allowPrivateUrls: true }`. Assert that `fetchContent` returns the rendered text with `method: "browser"`. Add tests that `--browser`/`forceBrowser` skips HTTP, `--no-browser` prevents fallback, short HTTP text triggers fallback, and non-HTML returns a structured error when browser fallback is disabled. Add a browser-navigation safety test where an allowed initial test URL redirects to `http://127.0.0.1:<port>/private`; browser fallback must abort before extracting private content and return `PRIVATE_URL_BLOCKED` or equivalent non-success status.

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/integration/browser-extract.test.ts
```

Expected: FAIL.

- [ ] **Step 3: Implement browser extraction**

Create `src/fetch/browser-extract.ts`:

- Validate the initial URL before browser navigation.
- Launch persistent browser context through `context.ts` with remaining deadline time.
- Intercept navigation requests and redirects where Playwright exposes them; validate each navigation URL with `validatePublicHttpUrl` before allowing it.
- Apply DNS resolver/private-IP checks where feasible before allowing browser navigation.
- Navigate with timeout based on `remainingMs(deadline)`.
- After navigation, validate `page.url()` before extraction to catch final private/local URLs.
- Wait for DOM content loaded and briefly for network idle only if deadline remains.
- Remove `script`, `style`, `nav`, `footer`, `aside`, and obvious cookie/banner nodes.
- Extract document title, common metadata, and visible body text.
- Truncate to `maxChars`.
- Return `FetchedContent` with `method: "browser"`.

Modify `src/fetch/fetch-content.ts`:

- Use HTTP first unless `forceBrowser` is set.
- Fall back to browser when HTTP extraction fails or text is too short.
- Honor `--no-browser` by returning HTTP error without fallback.

- [ ] **Step 4: Run browser extraction test**

Run:

```bash
bun test tests/integration/browser-extract.test.ts
```

Expected: PASS.

- [ ] **Step 5: Run all fetch-related tests**

Run:

```bash
bun test tests/unit/http-extract.test.ts tests/integration/fetch-content-cli.test.ts tests/integration/browser-extract.test.ts
```

Expected: PASS.

- [ ] **Step 6: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: add browser fallback content extraction"
```

Do not use `git add .`; do not suppress git failures.

### Task 9: Implement human intervention wait helper

**Files:**
- Create: `src/browser/human-intervention.ts`
- Create: `tests/unit/human-intervention.test.ts`

- [ ] **Step 1: Write failing helper tests**

Test pure timeout behavior by passing a fake `isResolved` callback that returns false until timeout. Assert `HUMAN_INTERVENTION_TIMEOUT`.

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/unit/human-intervention.test.ts
```

Expected: FAIL.

- [ ] **Step 3: Implement helper**

Create `waitForHumanIntervention({ page, timeoutMs, isResolved })`:

- Print instructions to stderr.
- Poll `isResolved` until true or timeout.
- Return `{ ok: true }` or `{ ok: false, error }`.
- Do not solve challenges automatically.

- [ ] **Step 4: Run test**

Run:

```bash
bun test tests/unit/human-intervention.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: add human intervention wait helper"
```

Do not use `git add .`; do not suppress git failures.

---

## Chunk 5: Google Search Parsing and CLI

### Task 10: Implement block and consent detection

**Files:**
- Create: `src/search/block-detection.ts`
- Create: `tests/fixtures/google-blocked.html`
- Create: `tests/fixtures/google-consent.html`
- Create: `tests/unit/block-detection.test.ts`

- [ ] **Step 1: Write blocked fixture and failing tests**

Blocked fixture should include text such as “Our systems have detected unusual traffic” and a `/sorry/` URL case. Consent fixture should include `consent.google.com`, “Before you continue to Google”, and consent button/form indicators. Tests assert detection returns recoverable blocked state with `HUMAN_INTERVENTION_REQUIRED` context.

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/unit/block-detection.test.ts
```

Expected: FAIL.

- [ ] **Step 3: Implement detection**

Create `detectGoogleBlock({ url, html, text })` that checks:

- `/sorry/` in URL.
- `consent.google.com` in URL.
- “Before you continue to Google” in text.
- consent form/button indicators.
- `unusual traffic` in page text.
- CAPTCHA/challenge indicators.
- Missing result containers plus challenge or consent text.

- [ ] **Step 4: Run tests**

Run:

```bash
bun test tests/unit/block-detection.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: detect google block and consent pages"
```

Do not use `git add .`; do not suppress git failures.

### Task 11: Implement Google result parser

**Files:**
- Create: `src/search/parse-results.ts`
- Create: `tests/fixtures/google-results.html`
- Create: `tests/unit/parse-results.test.ts`

- [ ] **Step 1: Write parser fixture and failing test**

Create a Google-like fixture with organic result containers plus ads/sponsored blocks, People Also Ask, Google internal links, `/url?q=...` redirect links, duplicate URLs, missing snippets, and malformed blocks. Test that parser returns unique organic external results with rank, title, URL, display URL, and optional snippet.

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/unit/parse-results.test.ts
```

Expected: FAIL.

- [ ] **Step 3: Implement parser**

Create `parseGoogleResults(html, limit)`:

- Use Cheerio.
- Prefer robust selectors around links with headings.
- Ignore ads, maps, videos, People Also Ask, and Google internal links.
- Normalize Google redirect URLs when possible.
- Deduplicate by final URL.
- Return up to `limit` results with 1-based ranks.

- [ ] **Step 4: Run parser tests**

Run:

```bash
bun test tests/unit/parse-results.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: parse google organic results"
```

Do not use `git add .`; do not suppress git failures.

### Task 12: Implement Google search orchestration

**Files:**
- Create: `src/search/google.ts`
- Create: `tests/unit/google-search.test.ts`

- [ ] **Step 1: Write orchestration tests with mocked page adapter**

Design `searchGoogle` so parsing and page loading can be tested with a fake adapter or injected HTML provider. Test:

- successful result response status `ok`
- blocked page with `--no-human` returns `blocked`
- consent page with `--no-human` returns `blocked`
- consent/challenge with human enabled invokes the intervention helper and uses visible/headful browser behavior
- profile lock failure returns `error` with `BROWSER_PROFILE_LOCKED`
- `--fetch` attaches fetched content for top N only
- partial fetch failures produce top-level `partial`
- pagination returns ranks 1-20 from two pages for `limit: 20`
- page 2 block returns available results plus a structured recoverable error
- slow fake phases respect the overall deadline and return `timeout`

- [ ] **Step 2: Run tests to verify failure**

Run:

```bash
bun test tests/unit/google-search.test.ts
```

Expected: FAIL.

- [ ] **Step 3: Implement orchestration**

Create `searchGoogle(query, options)`:

- Build Google search URL with encoded query.
- Acquire browser profile lock using the shared deadline.
- Launch persistent context headful when human intervention may be needed, or relaunch headful when a challenge is detected.
- Navigate to Google results with remaining deadline time.
- Detect block/consent/challenge.
- If blocked and `humanIntervention` is false, return `blocked`.
- If blocked and allowed, call human intervention helper and retry parse.
- Parse current page results and deduplicate globally.
- If parsed results `< limit`, cautiously paginate by following the next-page link until limit reached, no next page exists, deadline expires, or a block/challenge occurs.
- Preserve global 1-based ranks across pages.
- If `fetch` is true, call `fetchContent` for top `fetchLimit` results, passing the same deadline.
- Set status `partial` when any fetch fails while search succeeded.

- [ ] **Step 4: Run orchestration tests**

Run:

```bash
bun test tests/unit/google-search.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: orchestrate google browser search"
```

Do not use `git add .`; do not suppress git failures.

### Task 13: Implement `search-web` CLI

**Files:**
- Create: `src/cli/search-web.ts`
- Create: `tests/integration/search-web-cli.test.ts`

- [ ] **Step 1: Write CLI tests with mocked/injected search path if possible**

Test argument parsing and JSON/Markdown rendering without hitting Google. Prefer an environment variable fixture mode such as `SEARCH_HEADLESS_GOOGLE_FIXTURE=tests/fixtures/google-results.html` so CLI tests remain deterministic.

- [ ] **Step 2: Run test to verify failure**

Run:

```bash
bun test tests/integration/search-web-cli.test.ts
```

Expected: FAIL.

- [ ] **Step 3: Implement CLI**

Create `src/cli/search-web.ts`:

- Shebang `#!/usr/bin/env bun`.
- Commander arg `<query>`.
- Options for v0.1: `--limit`, `--fetch`, `--fetch-limit`, `--format`, `--headful`, `--no-human`, `--timeout`.
- Do not expose `--region` in v0.1; document it as deferred.
- Convert timeout to a shared deadline and pass it to `searchGoogle`.
- Call `searchGoogle`.
- Render JSON or Markdown.
- Exit 0 for `ok`/`partial`, non-zero otherwise.
- Add CLI tests for blocked fixture with `--no-human`, Markdown output, invalid format, partial fetch exit code 0, and stdout/stderr separation.

- [ ] **Step 4: Run CLI tests**

Run:

```bash
bun test tests/integration/search-web-cli.test.ts
```

Expected: PASS.

- [ ] **Step 5: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "feat: add search-web cli"
```

Do not use `git add .`; do not suppress git failures.

---

## Chunk 6: Documentation, Smoke Tests, and Final Verification

### Task 14: Write README

**Files:**
- Create: `README.md`

- [ ] **Step 1: Document usage**

Create `README.md` covering:

- Installation with Bun.
- `bun install` and Playwright browser installation if needed.
- Commands:
  - `search-web "query"`
  - `search-web "query" --fetch`
  - `fetch-content "url"`
- JSON vs Markdown output.
- Human intervention behavior.
- Safety boundary: no CAPTCHA bypass, no stealth, no proxy rotation.
- Exit codes.
- Private URL blocking.
- `--region` is intentionally deferred/not available in v0.1 because the spec marks it as later/optional.

- [ ] **Step 2: Optional commit checkpoint**

Only if the user explicitly requested commits and this directory is a git repo:

```bash
git status --short
# Stage only the intentional files for this task, then:
git commit -m "docs: document search-headless usage"
```

Do not use `git add .`; do not suppress git failures.

### Task 15: Final verification

**Files:**
- No new files expected.

- [ ] **Step 1: Install Playwright browser if needed**

Run:

```bash
bunx playwright install chromium
```

Expected: Chromium is available for Playwright.

- [ ] **Step 2: Run typecheck**

Run:

```bash
bun run typecheck
```

Expected: exit 0.

- [ ] **Step 3: Run full automated test suite**

Run:

```bash
bun test
```

Expected: all tests pass.

- [ ] **Step 4: Run fetch smoke test**

Run:

```bash
bun run src/cli/fetch-content.ts https://example.com --no-browser --max-chars 1000
```

Expected: exit 0, JSON stdout with `status: "ok"` and text from example.com.

- [ ] **Step 5: Run search smoke test**

Run:

```bash
bun run src/cli/search-web.ts "bun typescript" --limit 3 --no-human
```

Expected: either:

- exit 0 with JSON search results, or
- non-zero structured JSON with `status: "blocked"` if Google requires human intervention.

A `blocked` result is acceptable for `--no-human`; unstructured crashes are not acceptable.

- [ ] **Step 6: Review acceptance criteria**

Verify each acceptance criterion from `docs/superpowers/specs/2026-04-26-search-headless-design.md` is either implemented and tested, or explicitly documented as requiring manual smoke validation. Record `--region` as intentionally deferred for v0.1 because the spec marks it as later/optional.

- [ ] **Step 7: Report final working tree state**

Run:

```bash
git status --short
```

If the user explicitly requested commits and this is a git repo, stage only intentional files from this plan and commit without hiding failures. Otherwise, report the remaining changed files.
