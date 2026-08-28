import { cacheRead, cacheWrite, type CacheOptions } from "../cache";
import { DEFAULT_CONFIG } from "../config";
import { obscuraFetch, parseObscuraJson } from "../obscura/runner";
import { reserveSearchSlot } from "../rate-limit";
import type { Deadline, SearchResponse, SearchResult, ToolError } from "../types";
import { createDeadline, remainingMs } from "../types";
import { fetchManyContent } from "../fetch/fetch-content";
import { buildBraveSearchUrl, detectBraveBlock, parseBraveNextUrl, parseBraveResults } from "./parse-brave";

const CACHE_NAMESPACE = "search";

/** A page load that failed for a reason other than a challenge is retried once. */
const LOAD_ATTEMPTS = 2;
const RETRY_BACKOFF_MS = 1500;

export interface SearchBraveOptions {
  limit?: number;
  fetch?: boolean;
  fetchLimit?: number;
  deadline?: Deadline;
  timeoutMs?: number;
  allowPrivateUrls?: boolean;
  fixturePath?: string;
  cache?: CacheOptions;
  concurrency?: number;
}

/**
 * Hand the rendered page back for parsing here rather than picking results apart
 * in the page.
 *
 * Nothing is clicked, and the parse itself stays in TypeScript where the fixture
 * tests can reach it. Only inert weight is dropped before the HTML crosses the
 * pipe — a Brave SERP carries far more inline script than markup, and shipping
 * it costs more than the parse does. The footer stays, because Brave's "Next"
 * link lives there; `noscript` stays, because a JavaScript-required notice is
 * one of the interstitials that has to be detected.
 */
export const READ_SERP_SCRIPT = [
  `(() => {`,
  `  document.querySelectorAll('script,style,svg,link,iframe,template,picture,source').forEach(el => el.remove());`,
  `  return JSON.stringify({`,
  `    html: document.documentElement.outerHTML,`,
  `    text: document.body ? document.body.textContent || '' : '',`,
  `    url: window.location.href,`,
  `  });`,
  `})()`,
].join("");

interface SerpPage {
  html: string;
  text: string;
  url: string;
}

type SerpLoad =
  | { ok: true; page: SerpPage }
  | { ok: false; error: ToolError };

/**
 * Load one Brave SERP through obscura, retrying a transient failure once.
 *
 * A crashed or missing browser is not a bot challenge, and reporting it as one
 * sends an agent off to solve the wrong problem — so the two stay distinct all
 * the way out to the caller. A challenge is never retried: it is reported.
 */
async function loadSerp(url: string, deadline: Deadline): Promise<SerpLoad> {
  let last: ToolError = { code: "OBSCURA_SEARCH_FAILED", message: "obscura produced no output.", recoverable: true };

  for (let attempt = 1; attempt <= LOAD_ATTEMPTS; attempt++) {
    if (remainingMs(deadline) <= 1000) {
      return { ok: false, error: { code: "FETCH_TIMEOUT", message: "Overall timeout expired before the results page loaded.", recoverable: true } };
    }

    const result = await obscuraFetch({ url, deadline, evalScript: READ_SERP_SCRIPT, stealth: true });
    if (result.ok) {
      const page = parseObscuraJson<SerpPage>(result.stdout);
      if (page && typeof page.html === "string") return { ok: true, page };
      last = { code: "OBSCURA_SEARCH_FAILED", message: "obscura returned no readable results page.", recoverable: true };
    } else if (remainingMs(deadline) <= 0) {
      return { ok: false, error: { code: "FETCH_TIMEOUT", message: "Overall timeout expired while loading the results page.", recoverable: true } };
    } else {
      last = {
        code: "OBSCURA_SEARCH_FAILED",
        message: result.stderr.trim() || `obscura exited with code ${result.exitCode}.`,
        recoverable: true,
      };
    }

    if (attempt < LOAD_ATTEMPTS && remainingMs(deadline) > RETRY_BACKOFF_MS + 1000) {
      await Bun.sleep(RETRY_BACKOFF_MS);
    }
  }

  return { ok: false, error: last };
}

function response(query: string, status: SearchResponse["status"], results: SearchResult[], errors: ToolError[]): SearchResponse {
  return { query, engine: "brave", status, results, errors };
}

function blockedResponse(query: string, message: string, results: SearchResult[] = []): SearchResponse {
  return response(query, "blocked", results, [{ code: "BRAVE_CHALLENGE", message, recoverable: true }]);
}

/** Read SERP pages from a fixture instead of the network, for tests. */
async function loadFixtureSerp(fixturePath: string): Promise<SerpPage[]> {
  const html = await Bun.file(fixturePath).text();
  const pages: SerpPage[] = [{ html, text: html, url: "https://search.brave.com/search" }];
  const second = process.env.SEARCH_HEADLESS_BRAVE_FIXTURE_2;
  if (second) {
    const html2 = await Bun.file(second).text();
    pages.push({ html: html2, text: html2, url: "https://search.brave.com/search?offset=1" });
  }
  return pages;
}

/**
 * Main Brave Search function.
 *
 * Obscura is the only engine. If Brave serves a challenge instead of results,
 * that is reported as `status: "blocked"` — never as an empty success, and never
 * worked around.
 */
export async function searchBrave(query: string, options: SearchBraveOptions = {}): Promise<SearchResponse> {
  const limit = options.limit ?? DEFAULT_CONFIG.searchLimit;
  const deadline = options.deadline ?? createDeadline(options.timeoutMs ?? DEFAULT_CONFIG.timeoutMs);
  const fixture = options.fixturePath ?? process.env.SEARCH_HEADLESS_BRAVE_FIXTURE;
  const cacheOptions: CacheOptions = { ttlMs: DEFAULT_CONFIG.searchCacheTtlMs, ...options.cache };
  const errors: ToolError[] = [];

  try {
    const fixturePages = fixture ? await loadFixtureSerp(fixture) : null;

    // A repeated query is answered from disk, which also spares the rate limit.
    const cached = fixturePages ? undefined : cacheRead<SearchResponse>(CACHE_NAMESPACE, `${limit}:${query}`, cacheOptions);
    const base = cached ?? (await collect(query, limit, deadline, fixturePages, errors));
    if (base.status === "blocked" || base.status === "error" || base.status === "timeout") return base;

    if (!cached) cacheWrite(CACHE_NAMESPACE, `${limit}:${query}`, base, cacheOptions);
    const result: SearchResponse = cached ? { ...cached, cached: true } : base;

    if (!options.fetch || result.results.length === 0) return result;

    // Content fetches have their own cache and their own failure modes; a bad
    // fetch downgrades an otherwise good search to `partial`, never to `blocked`.
    const n = Math.min(options.fetchLimit ?? DEFAULT_CONFIG.fetchLimit, result.results.length);
    const targets = result.results.slice(0, n);
    const fetched = await fetchManyContent(
      targets.map((r) => r.url),
      { deadline, allowPrivateUrls: options.allowPrivateUrls, cache: options.cache, concurrency: options.concurrency },
    );
    let fetchFailed = false;
    fetched.forEach((content, i) => {
      targets[i].fetched = content;
      if (content.status !== "ok") fetchFailed = true;
    });

    return { ...result, status: fetchFailed || result.errors.length > 0 ? "partial" : result.status };
  } catch (e) {
    const err = e as ToolError;
    const expired = remainingMs(deadline) <= 0;
    return response(query, expired ? "timeout" : "error", [], [{
      code: err?.code ?? (expired ? "FETCH_TIMEOUT" : "UNKNOWN_ERROR"),
      message: err?.message || String(e),
      recoverable: err?.recoverable ?? true,
    }]);
  }
}

/** Load and parse SERP pages until the limit is met or Brave stops cooperating. */
async function collect(
  query: string,
  limit: number,
  deadline: Deadline,
  fixturePages: SerpPage[] | null,
  errors: ToolError[],
): Promise<SearchResponse> {
  let first: SerpPage;
  if (fixturePages) {
    const page = fixturePages[0];
    if (!page) return blockedResponse(query, "Brave Search did not return a readable results page.");
    first = page;
  } else {
    const slot = await reserveSearchSlot({ deadline });
    if (!slot.ok) {
      return response(query, "timeout", [], [{
        code: "FETCH_TIMEOUT",
        message: `Rate limiting needs another ${Math.ceil(slot.requiredMs / 1000)}s, which is longer than the remaining timeout.`,
        recoverable: true,
      }]);
    }
    const load = await loadSerp(buildBraveSearchUrl(query), deadline);
    if (!load.ok) {
      return response(query, load.error.code === "FETCH_TIMEOUT" ? "timeout" : "error", [], [load.error]);
    }
    first = load.page;
  }

  if (detectBraveBlock(first)) {
    return blockedResponse(query, "Brave Search served a bot challenge instead of results.");
  }

  const allResults: SearchResult[] = [];
  const seen = new Set<string>();

  const add = (serp: SerpPage) => {
    for (const result of parseBraveResults(serp.html, limit)) {
      const key = canonicalUrl(result.url);
      if (seen.has(key) || allResults.length >= limit) continue;
      seen.add(key);
      result.rank = allResults.length + 1;
      allResults.push(result);
    }
  };

  add(first);

  // Follow pagination until the limit is met.
  let nextUrl = parseBraveNextUrl(first.html);
  let fixtureIndex = 1;

  while (allResults.length < limit && remainingMs(deadline) > 2000) {
    let serp: SerpPage | null = null;
    if (fixturePages) {
      serp = fixturePages[fixtureIndex++] ?? null;
    } else if (nextUrl) {
      const slot = await reserveSearchSlot({ deadline });
      if (!slot.ok) break;
      const load = await loadSerp(nextUrl, deadline);
      if (!load.ok) {
        errors.push(load.error);
        break;
      }
      serp = load.page;
    }
    if (!serp) break;

    if (detectBraveBlock(serp)) {
      // A challenge outranks a partial page: a later --fetch problem must not
      // downgrade "blocked" to "partial" and turn a non-zero exit into success.
      return blockedResponse(query, "Brave Search served a bot challenge while paginating.", allResults);
    }

    const before = allResults.length;
    add(serp);
    nextUrl = parseBraveNextUrl(serp.html);
    if (allResults.length === before) break; // no progress
  }

  return response(query, errors.length > 0 ? "partial" : "ok", allResults, [...errors]);
}

/**
 * Key used to spot the same page twice across SERP pages.
 *
 * Brave repeats a result between pages with a different tracking suffix often
 * enough that exact string matching lets duplicates through.
 */
export function canonicalUrl(input: string): string {
  try {
    const url = new URL(input);
    url.hash = "";
    for (const key of [...url.searchParams.keys()]) {
      if (/^(utm_|ref$|ref_|fbclid$|gclid$|mc_[ce]id$|igshid$|spm$)/i.test(key)) url.searchParams.delete(key);
    }
    const path = url.pathname.replace(/\/+$/, "");
    return `${url.protocol}//${url.hostname.replace(/^www\./, "")}${path}${url.search}`.toLowerCase();
  } catch {
    return input;
  }
}
