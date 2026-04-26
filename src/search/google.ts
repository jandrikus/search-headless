import { DEFAULT_CONFIG } from "../config";
import { withProfileLock } from "../browser/context";
import { waitForHumanIntervention } from "../browser/human-intervention";
import type { Deadline, SearchResponse, ToolError } from "../types";
import { createDeadline, remainingMs } from "../types";
import { fetchContent } from "../fetch/fetch-content";
import { detectGoogleBlock } from "./block-detection";
import { parseGoogleNextUrl, parseGoogleResults } from "./parse-results";
import { chromium } from "playwright";

export interface SearchGoogleOptions { limit?: number; fetch?: boolean; fetchLimit?: number; noHuman?: boolean; headful?: boolean; deadline?: Deadline; timeoutMs?: number; allowPrivateUrls?: boolean; fixturePath?: string; }

export async function searchGoogle(query: string, options: SearchGoogleOptions = {}): Promise<SearchResponse> {
  const limit = options.limit ?? DEFAULT_CONFIG.searchLimit;
  const deadline = options.deadline ?? createDeadline(options.timeoutMs ?? DEFAULT_CONFIG.timeoutMs);
  const fixture = options.fixturePath ?? process.env.SEARCH_HEADLESS_GOOGLE_FIXTURE;
  try {
    const pages = fixture ? await loadFixturePages(fixture) : await loadBrowserPages(query, limit, options, deadline);
    const firstBlock = detectGoogleBlock({ url: pages[0]?.url ?? "https://www.google.com", html: pages[0]?.html ?? "", text: pages[0]?.text ?? "" });
    if (firstBlock) return { query, engine: "google", status: "blocked", results: [], errors: [firstBlock] };

    const errors: ToolError[] = [];
    const seen = new Set<string>();
    const results = [];
    for (const pageData of pages) {
      const block = detectGoogleBlock({ url: pageData.url, html: pageData.html, text: pageData.text });
      if (block) { errors.push(block); break; }
      for (const result of parseGoogleResults(pageData.html, limit, 1)) {
        if (seen.has(result.url) || seen.size >= limit) continue;
        seen.add(result.url); result.rank = seen.size; results.push(result);
      }
      if (results.length >= limit) break;
    }
    const hasHumanRequiredError = errors.some((error) => error.code === "HUMAN_INTERVENTION_REQUIRED");
    const response: SearchResponse = { query, engine: "google", status: hasHumanRequiredError && options.noHuman ? "blocked" : errors.length ? "partial" : "ok", results, errors };
    if (options.fetch) {
      let partial = response.status === "partial";
      const n = Math.min(options.fetchLimit ?? DEFAULT_CONFIG.fetchLimit, results.length);
      for (let i = 0; i < n; i++) {
        results[i].fetched = await fetchContent(results[i].url, { deadline, allowPrivateUrls: options.allowPrivateUrls });
        if (results[i].fetched!.status !== "ok") partial = true;
      }
      if (partial) response.status = "partial";
    }
    return response;
  } catch (e) {
    const err = e as ToolError;
    if (err?.code === "BROWSER_PROFILE_LOCKED") return { query, engine: "google", status: "error", results: [], errors: [err] };
    return { query, engine: "google", status: remainingMs(deadline) <= 0 ? "timeout" : "error", results: [], errors: [{ code: remainingMs(deadline) <= 0 ? "FETCH_TIMEOUT" : "UNKNOWN_ERROR", message: e instanceof Error ? e.message : String(e), recoverable: true }] };
  }
}

async function loadFixturePages(fixture: string) {
  const pages = [{ url: "https://www.google.com/search", html: await Bun.file(fixture).text(), text: "" }];
  pages[0].text = pages[0].html;
  if (process.env.SEARCH_HEADLESS_GOOGLE_FIXTURE_2) {
    const html = await Bun.file(process.env.SEARCH_HEADLESS_GOOGLE_FIXTURE_2).text();
    pages.push({ url: "https://www.google.com/search?start=10", html, text: html });
  }
  return pages;
}

async function loadBrowserPages(query: string, limit: number, options: SearchGoogleOptions, deadline: Deadline) {
  return withProfileLock(DEFAULT_CONFIG.lockFile, deadline, async () => {
    const pages: Array<{ url: string; html: string; text: string }> = [];
    const context = await chromium.launchPersistentContext(DEFAULT_CONFIG.profileDir, { headless: !(options.headful || !options.noHuman) });
    try {
      const page = await context.newPage();
      let url = "https://www.google.com/search?q=" + encodeURIComponent(query);
      await page.goto(url, { waitUntil: "domcontentloaded", timeout: Math.max(1, remainingMs(deadline)) });
      for (;;) {
        const html = await page.content();
        const text = await page.locator("body").innerText().catch(() => html);
        url = page.url();
        const block = detectGoogleBlock({ url, html, text });
        if (block && !options.noHuman) {
          const wait = await waitForHumanIntervention({ deadline, isResolved: async () => {
            try {
              const currentHtml = await page.content();
              const currentText = await page.locator("body").innerText().catch(() => currentHtml);
              return !detectGoogleBlock({ url: page.url(), html: currentHtml, text: currentText });
            } catch {
              return false;
            }
          }});
          if (!wait.ok) { pages.push({ url, html, text }); break; }
          continue;
        }
        pages.push({ url, html, text });
        if (pages.length >= Math.ceil(limit / 10)) break;
        const next = parseGoogleNextUrl(html);
        if (!next || remainingMs(deadline) <= 0) break;
        await page.goto(next, { waitUntil: "domcontentloaded", timeout: Math.max(1, remainingMs(deadline)) });
      }
      return pages;
    } finally { await context.close().catch(() => undefined); }
  });
}
