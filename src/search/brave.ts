import { DEFAULT_CONFIG } from "../config";
import type { Deadline, SearchResponse, SearchResult, ToolError } from "../types";
import { createDeadline, remainingMs } from "../types";
import { fetchContent } from "../fetch/fetch-content";
import { buildBraveSearchUrl, detectBraveBlock, parseBraveNextUrl, parseBraveResults } from "./parse-brave";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** Minimum seconds between searches to appear human-like. */
const MIN_SEARCH_INTERVAL_SEC = 10;

/** Path to the last-search timestamp file (in /tmp for speed). */
const LAST_SEARCH_FILE = "/tmp/search-headless-last-search";

/**
 * Rate limiter: wait until enough time has passed since the last search.
 * Uses a file in /tmp (RAM) to persist timestamp across CLI invocations.
 */
async function waitForRateLimit(): Promise<void> {
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

// Eval script for Obscura to extract Brave results directly from DOM
const BRAVE_EVAL_SCRIPT = [
  `(() => {`,
  `  document.querySelectorAll('script,style,nav,footer,aside').forEach(el => el.remove());`,
  `  const snippets = document.querySelectorAll('.snippet[data-type="web"]:not(.standalone)');`,
  `  const results = Array.from(snippets).map((s, i) => {`,
  `    const link = s.querySelector('.result-content a');`,
  `    const titleEl = s.querySelector('.title');`,
  `    const title = titleEl?.getAttribute('title') || titleEl?.textContent?.trim() || '';`,
  `    const url = link?.href || '';`,
  `    const displayUrl = s.querySelector('cite.snippet-url')?.textContent?.trim() || '';`,
  `    const snippet = s.querySelector('.generic-snippet .content')?.textContent?.trim() || '';`,
  `    return { rank: i + 1, title, url, displayUrl, snippet };`,
  `  }).filter(r => r.url && r.title);`,
  `  const nextLink = Array.from(document.querySelectorAll('a')).find(a => a.textContent?.trim() === 'Next');`,
  `  const nextUrl = nextLink?.href || null;`,
  `  return JSON.stringify({ results, nextUrl, blocked: false });`,
  `})()`,
].join("");

// Eval script to check if page is blocked by CAPTCHA
const BRAVE_BLOCK_CHECK_SCRIPT = [
  `(() => {`,
  `  const text = document.body?.innerText || '';`,
  `  const blocked = text.includes('Verifying you') || text.includes('Verify you are human');`,
  `  return JSON.stringify({ blocked });`,
  `})()`,
].join("");

/**
 * Search Brave using Obscura (fast, low-memory headless browser).
 * Returns null if CAPTCHA is detected (caller should fall back to Playwright).
 */
async function searchBraveWithObscura(
  query: string,
  limit: number,
  deadline: Deadline,
): Promise<{ results: SearchResult[]; nextUrl: string | null; blocked: boolean } | null> {
  const timeout = remainingMs(deadline);
  if (timeout <= 10) return null;

  const url = buildBraveSearchUrl(query);
  const waitSecs = Math.max(1, Math.min(10, Math.ceil(timeout / 1000)));

  try {
    const proc = Bun.spawn(
      [
        "obscura", "fetch", url,
        "--quiet", "--stealth",
        "--wait", String(waitSecs),
        "--timeout", String(Math.max(1, Math.ceil(timeout / 1000))),
        "--eval", BRAVE_EVAL_SCRIPT,
      ],
      { stdout: "pipe", stderr: "pipe", timeout: Math.max(1000, timeout) },
    );

    const stdout = await new Response(proc.stdout).text();
    const exitCode = await proc.exited;

    if (exitCode !== 0 || !stdout.trim()) return null;

    const parsed = JSON.parse(stdout.trim());
    if (parsed.blocked) return null;

    return {
      results: parsed.results || [],
      nextUrl: parsed.nextUrl || null,
      blocked: false,
    };
  } catch {
    return null;
  }
}

/**
 * Search Brave using Playwright (full browser with CAPTCHA solving).
 * Used as fallback when Obscura hits a CAPTCHA.
 */
async function searchBraveWithPlaywright(
  query: string,
  limit: number,
  deadline: Deadline,
): Promise<{ results: SearchResult[]; nextUrl: string | null }> {
  const { chromium } = await import("playwright");

  const timeout = remainingMs(deadline);
  if (timeout <= 10) return { results: [], nextUrl: null };

  const browser = await chromium.launch({ headless: true });
  try {
    const context = await browser.newContext({
      userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
      extraHTTPHeaders: { "Accept-Language": "en-US,en;q=0.9" },
    });

    // Set cookies for English + All regions
    await context.addCookies([
      { name: "country", value: "all", domain: ".search.brave.com", path: "/" },
      { name: "ui_lang", value: "en-us", domain: ".search.brave.com", path: "/" },
    ]);

    const page = await context.newPage();
    const url = buildBraveSearchUrl(query);

    await page.goto(url, {
      waitUntil: "domcontentloaded",
      timeout: Math.max(1, remainingMs(deadline)),
    });

    // Wait for content to render
    await page.waitForTimeout(3000);

    // Check for CAPTCHA and solve it
    const blocked = await page.evaluate(() => {
      const text = document.body?.innerText || "";
      return text.includes("Verifying you") || text.includes("Verify you are human");
    });

    if (blocked) {
      // Click the Verify button
      const verifyBtn = page.locator('button:has-text("Verify")');
      if (await verifyBtn.isVisible().catch(() => false)) {
        await verifyBtn.click();
        // Wait for verification to complete
        await page.waitForTimeout(10000);
      }
    }

    // Extract results using the eval script
    const extracted = await page.evaluate(() => {
      document.querySelectorAll('script,style,nav,footer,aside').forEach(el => el.remove());
      const snippets = document.querySelectorAll('.snippet[data-type="web"]:not(.standalone)');
      const results = Array.from(snippets).map((s, i) => {
        const link = s.querySelector('.result-content a') as HTMLAnchorElement | null;
        const titleEl = s.querySelector('.title');
        const title = titleEl?.getAttribute('title') || titleEl?.textContent?.trim() || '';
        const url = link?.href || '';
        const displayUrl = s.querySelector('cite.snippet-url')?.textContent?.trim() || '';
        const snippet = s.querySelector('.generic-snippet .content')?.textContent?.trim() || '';
        return { rank: i + 1, title, url, displayUrl, snippet };
      }).filter(r => r.url && r.title);
      const nextLink = Array.from(document.querySelectorAll('a')).find(a => a.textContent?.trim() === 'Next');
      const nextUrl = (nextLink as HTMLAnchorElement)?.href || null;
      return JSON.stringify({ results, nextUrl, blocked: false });
    }).catch(() => null);
    if (!extracted) return { results: [], nextUrl: null };

    const parsed = JSON.parse(extracted);
    return {
      results: parsed.results || [],
      nextUrl: parsed.nextUrl || null,
    };
  } finally {
    await browser.close().catch(() => undefined);
  }
}

/**
 * Main Brave Search function.
 * Tries Obscura first, falls back to Playwright for CAPTCHA solving.
 */
export async function searchBrave(query: string, options: SearchBraveOptions = {}): Promise<SearchResponse> {
  const limit = options.limit ?? DEFAULT_CONFIG.searchLimit;
  const deadline = options.deadline ?? createDeadline(options.timeoutMs ?? DEFAULT_CONFIG.timeoutMs);
  const errors: ToolError[] = [];

  // Rate limit: wait if last search was too recent
  await waitForRateLimit();

  try {
    // Try Obscura first (fast, low memory)
    let obscuraResult = await searchBraveWithObscura(query, limit, deadline);

    // If Obscura hit a CAPTCHA, fall back to Playwright
    if (obscuraResult === null || obscuraResult.blocked) {
      errors.push({
        code: "OBSCURA_CAPTCHA_FALLBACK",
        message: "Obscura encountered a CAPTCHA, falling back to Playwright.",
        recoverable: true,
      });

      const playwrightResult = await searchBraveWithPlaywright(query, limit, deadline);
      obscuraResult = { ...playwrightResult, blocked: false };
    }

    // Collect results, handling pagination
    const allResults: SearchResult[] = [];
    const seen = new Set<string>();

    // Add first page results
    for (const result of obscuraResult.results) {
      if (seen.has(result.url) || allResults.length >= limit) continue;
      seen.add(result.url);
      result.rank = allResults.length + 1;
      allResults.push(result);
    }

    // Fetch additional pages if needed
    let nextUrl = obscuraResult.nextUrl;
    while (allResults.length < limit && nextUrl && remainingMs(deadline) > 0) {
      // For pagination, we use Obscura (Playwright fallback would be too slow for multiple pages)
      const pageResult = await fetchBravePage(nextUrl, deadline);
      if (!pageResult || pageResult.results.length === 0) break;

      for (const result of pageResult.results) {
        if (seen.has(result.url) || allResults.length >= limit) continue;
        seen.add(result.url);
        result.rank = allResults.length + 1;
        allResults.push(result);
      }
      nextUrl = pageResult.nextUrl;
    }

    const response: SearchResponse = {
      query,
      engine: "brave",
      status: errors.length > 0 ? "partial" : "ok",
      results: allResults,
      errors,
    };

    // Optionally fetch content for top results
    if (options.fetch) {
      let partial = response.status === "partial";
      const n = Math.min(options.fetchLimit ?? DEFAULT_CONFIG.fetchLimit, allResults.length);
      for (let i = 0; i < n; i++) {
        allResults[i].fetched = await fetchContent(allResults[i].url, {
          deadline,
          allowPrivateUrls: options.allowPrivateUrls,
        });
        if (allResults[i].fetched!.status !== "ok") partial = true;
      }
      if (partial) response.status = "partial";
    }

    return response;
  } catch (e) {
    const err = e as ToolError;
    return {
      query,
      engine: "brave",
      status: remainingMs(deadline) <= 0 ? "timeout" : "error",
      results: [],
      errors: [
        {
          code: remainingMs(deadline) <= 0 ? "FETCH_TIMEOUT" : "UNKNOWN_ERROR",
          message: err?.message || String(e),
          recoverable: true,
        },
      ],
    };
  }
}

/**
 * Fetch a single Brave Search results page using Obscura.
 */
async function fetchBravePage(
  url: string,
  deadline: Deadline,
): Promise<{ results: SearchResult[]; nextUrl: string | null } | null> {
  const timeout = remainingMs(deadline);
  if (timeout <= 10) return null;

  const waitSecs = Math.max(1, Math.min(10, Math.ceil(timeout / 1000)));

  try {
    const proc = Bun.spawn(
      [
        "obscura", "fetch", url,
        "--quiet", "--stealth",
        "--wait", String(waitSecs),
        "--timeout", String(Math.max(1, Math.ceil(timeout / 1000))),
        "--eval", BRAVE_EVAL_SCRIPT,
      ],
      { stdout: "pipe", stderr: "pipe", timeout: Math.max(1000, timeout) },
    );

    const stdout = await new Response(proc.stdout).text();
    const exitCode = await proc.exited;

    if (exitCode !== 0 || !stdout.trim()) return null;

    const parsed = JSON.parse(stdout.trim());
    if (parsed.blocked) return null;

    return {
      results: parsed.results || [],
      nextUrl: parsed.nextUrl || null,
    };
  } catch {
    return null;
  }
}
