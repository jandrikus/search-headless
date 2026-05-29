import * as cheerio from "cheerio";
import type { SearchResult } from "../types";

/**
 * Parse Brave Search SERP HTML into structured results.
 *
 * Brave uses Svelte-based DOM with these selectors:
 * - Container: .snippet[data-type="web"]:not(.standalone)
 * - Link: .result-content a[href]
 * - Title: .title (has title="" attribute with clean text)
 * - Display URL: cite.snippet-url
 * - Description: .generic-snippet .content
 */
export function parseBraveResults(html: string, limit = 10, startRank = 1): SearchResult[] {
  const $ = cheerio.load(html);
  const out: SearchResult[] = [];
  const seen = new Set<string>();

  // Each organic result is in a .snippet div with data-type="web"
  $(".snippet[data-type='web']").each((_, snippet) => {
    if (out.length >= limit) return false;

    const $snippet = $(snippet);

    // Skip standalone snippets (videos, discussions, etc.)
    if ($snippet.hasClass("standalone")) return;

    // Get the main link
    const $link = $snippet.find(".result-content a[href]").first();
    const href = $link.attr("href");
    if (!href || !href.startsWith("http")) return;

    // Skip if already seen
    if (seen.has(href)) return;
    seen.add(href);

    // Extract title - prefer the title="" attribute (cleaner), fall back to text
    const titleEl = $snippet.find(".title").first();
    const title = titleEl.attr("title") || titleEl.text().trim();
    if (!title) return;

    // Extract display URL from cite element
    const displayUrl = $snippet.find("cite.snippet-url").text().trim() || undefined;

    // Extract snippet/description
    const snippet_text = $snippet.find(".generic-snippet .content").text().trim() || undefined;

    out.push({
      rank: startRank + out.length,
      title,
      url: href,
      displayUrl,
      snippet: snippet_text,
    });
  });

  return out;
}

/**
 * Parse the "Next" page URL from Brave Search HTML.
 * Brave uses a link with text "Next" that contains &offset=N.
 */
export function parseBraveNextUrl(html: string): string | undefined {
  const $ = cheerio.load(html);

  // Find the "Next" link
  let nextHref: string | undefined;
  $("a").each((_, a) => {
    const text = $(a).text().trim();
    if (text === "Next") {
      nextHref = $(a).attr("href");
      return false; // break
    }
  });

  if (!nextHref) return undefined;

  // Make absolute URL if needed
  try {
    return new URL(nextHref, "https://search.brave.com").href;
  } catch {
    return undefined;
  }
}

/**
 * Detect if the Brave Search page is showing a CAPTCHA.
 * Brave shows "Verifying you're not a bot" with a "Verify" button.
 */
export function detectBraveBlock({ url, html, text }: { url: string; html: string; text: string }): boolean {
  const lowerText = text.toLowerCase();
  return lowerText.includes("verifying you") || lowerText.includes("verify you are human");
}

/**
 * Build a Brave Search URL with the given query and options.
 */
export function buildBraveSearchUrl(query: string, options: { offset?: number } = {}): string {
  const params = new URLSearchParams({ q: query });
  if (options.offset) {
    params.set("offset", String(options.offset));
  }
  return `https://search.brave.com/search?${params.toString()}`;
}
