import { DEFAULT_CONFIG } from "../config";
import { obscuraFetch, parseObscuraJson } from "../obscura/runner";
import type { Deadline, SearchResponse, SearchResult, ToolError } from "../types";
import { createDeadline, remainingMs } from "../types";
import { fetchManyContent } from "../fetch/fetch-content";
import { buildBraveSearchUrl, detectBraveBlock, parseBraveNextUrl, parseBraveResults } from "./parse-brave";
import { existsSync, readFileSync, writeFileSync } from "node:fs";

/** Minimum seconds between searches to appear human-like. */
const MIN_SEARCH_INTERVAL_SEC = 10;

/** Path to the last-search timestamp file (in /tmp for speed). */
const LAST_SEARCH_FILE = "/tmp/search-headless-last-search";

/**
 * Rate limiter: wait until enough time has passed since the last search.
 * Uses a file in /tmp (RAM) to persist timestamp across CLI invocations.
 */
async function waitForRateLimit(): Promise<void> {
  if (process.env.SEARCH_HEADLESS_NO_RATE_LIMIT === "1") return;
  try {
    if (existsSync(LAST_SEARCH_FILE)) {
      const lastSearch = parseInt(readFileSync(LAST_SEARCH_FILE, "utf-8").trim(), 10);
      if (!isNaN(lastSearch)) {
        const elapsed = (Date.now() - lastSearch) / 1000;
        const remaining = MIN_SEARCH_INTERVAL_SEC - elapsed;
        if (remaining > 0) {
          // Wait the remaining time, rounded up to nearest 100ms
          await Bun.sleep(Math.ceil(remaining * 1000));
        }
      }
    }
  } catch {
    // Ignore errors reading the file
  }
  // Update the timestamp
  try {
    writeFileSync(LAST_SEARCH_FILE, String(Date.now()));
  } catch {
    // Ignore errors writing the file
  }
}

export interface SearchBraveOptions {
  limit?: number;
  fetch?: boolean;
  fetchLimit?: number;
  deadline?: Deadline;
  timeoutMs?: number;
  allowPrivateUrls?: boolean;
  fixturePath?: string;
}

/**
 * Hand the rendered page back for parsing here rather than picking results apart
 * in the page.
 *
 * Nothing is stripped and nothing is clicked. Stripping in-page is what breaks
 * pagination, because Brave's "Next" link sits in the footer; and the project
 * does not work around challenges, so a verify control is left alone and
 * reported instead.
 */
export const READ_SERP_SCRIPT = [
  `(() => JSON.stringify({`,
  `  html: document.documentElement.outerHTML,`,
  `  text: document.body ? document.body.textContent || '' : '',`,
  `  url: window.location.href,`,
  `}))()`,
].join("");

interface SerpPage {
  html: string;
  text: string;
  url: string;
}

/** Load one Brave SERP through obscura. */
async function loadSerp(url: string, deadline: Deadline): Promise<SerpPage | null> {
  const result = await obscuraFetch({ url, deadline, evalScript: READ_SERP_SCRIPT, stealth: true });
  if (!result.ok) return null;
  return parseObscuraJson<SerpPage>(result.stdout);
}

function blockedResponse(query: string, message: string, results: SearchResult[] = []): SearchResponse {
  return {
    query,
    engine: "brave",
    status: "blocked",
    results,
    errors: [{ code: "BRAVE_CHALLENGE", message, recoverable: true }],
  };
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
  const errors: ToolError[] = [];

  try {
    const fixturePages = fixture ? await loadFixtureSerp(fixture) : null;

    let first: SerpPage | null;
    if (fixturePages) {
      first = fixturePages[0] ?? null;
    } else {
      await waitForRateLimit();
      first = await loadSerp(buildBraveSearchUrl(query), deadline);
    }

    if (!first) return blockedResponse(query, "Brave Search did not return a readable results page.");
    if (detectBraveBlock(first)) {
      return blockedResponse(query, "Brave Search served a bot challenge instead of results.");
    }

    const allResults: SearchResult[] = [];
    const seen = new Set<string>();

    const collect = (serp: SerpPage) => {
      for (const result of parseBraveResults(serp.html, limit)) {
        if (seen.has(result.url) || allResults.length >= limit) continue;
        seen.add(result.url);
        result.rank = allResults.length + 1;
        allResults.push(result);
      }
    };

    collect(first);

    // Follow pagination until the limit is met.
    let nextUrl = parseBraveNextUrl(first.html);
    let fixtureIndex = 1;
    let blockedWhilePaginating = false;

    while (allResults.length < limit && remainingMs(deadline) > 2000) {
      let serp: SerpPage | null = null;
      if (fixturePages) {
        serp = fixturePages[fixtureIndex++] ?? null;
      } else if (nextUrl) {
        serp = await loadSerp(nextUrl, deadline);
      }
      if (!serp) break;

      if (detectBraveBlock(serp)) {
        blockedWhilePaginating = true;
        errors.push({
          code: "BRAVE_CHALLENGE",
          message: "Brave Search served a bot challenge while paginating.",
          recoverable: true,
        });
        break;
      }

      const before = allResults.length;
      collect(serp);
      nextUrl = parseBraveNextUrl(serp.html);
      if (allResults.length === before) break; // no progress
    }

    // Optionally fetch content for top results, concurrently.
    let fetchFailed = false;
    if (options.fetch && allResults.length > 0) {
      const n = Math.min(options.fetchLimit ?? DEFAULT_CONFIG.fetchLimit, allResults.length);
      const targets = allResults.slice(0, n);
      const fetched = await fetchManyContent(
        targets.map((r) => r.url),
        { deadline, allowPrivateUrls: options.allowPrivateUrls },
      );
      fetched.forEach((content, i) => {
        targets[i].fetched = content;
        if (content.status !== "ok") fetchFailed = true;
      });
    }

    // A challenge outranks a fetch failure: a later --fetch problem must not
    // downgrade "blocked" to "partial" and turn a non-zero exit into success.
    if (blockedWhilePaginating) {
      return { query, engine: "brave", status: "blocked", results: allResults, errors };
    }

    return {
      query,
      engine: "brave",
      status: errors.length > 0 || fetchFailed ? "partial" : "ok",
      results: allResults,
      errors,
    };
  } catch (e) {
    const err = e as ToolError;
    return {
      query,
      engine: "brave",
      status: remainingMs(deadline) <= 0 ? "timeout" : "error",
      results: [],
      errors: [
        {
          code: err?.code ?? (remainingMs(deadline) <= 0 ? "FETCH_TIMEOUT" : "UNKNOWN_ERROR"),
          message: err?.message || String(e),
          recoverable: err?.recoverable ?? true,
        },
      ],
    };
  }
}
