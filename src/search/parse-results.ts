import * as cheerio from "cheerio";
import type { SearchResult } from "../types";
export function parseGoogleResults(html: string, limit = 10, startRank = 1): SearchResult[] {
  const $ = cheerio.load(html); const out: SearchResult[] = []; const seen = new Set<string>();
  $("a:has(h3)").each((_, a) => {
    if (out.length >= limit) return false;
    const title = $(a).find("h3").first().text().trim(); if (!title) return;
    const raw = $(a).attr("href") || ""; const url = normalizeGoogleUrl(raw); if (!url || isGoogleInternal(url) || seen.has(url)) return;
    seen.add(url);
    const container = $(a).closest("div.g, div");
    const snippet = container.find(".VwiC3b").first().text().trim() || undefined;
    const displayUrl = container.find("cite").first().text().trim() || undefined;
    out.push({ rank: startRank + out.length, title, url, displayUrl, snippet });
  });
  return out;
}
function normalizeGoogleUrl(raw: string): string | undefined { try { const u = new URL(raw, "https://www.google.com"); if (u.pathname === "/url" && u.searchParams.get("q")) return u.searchParams.get("q")!; return u.href; } catch { return; } }
export function parseGoogleNextUrl(html: string): string | undefined {
  const $ = cheerio.load(html);
  const href = $("a#pnnext").attr("href") || $("a[aria-label='Next page']").attr("href");
  if (!href) return undefined;
  try { return new URL(href, "https://www.google.com").href; } catch { return undefined; }
}
function isGoogleInternal(url: string): boolean { try { const h = new URL(url).hostname; return h === "google.com" || h.endsWith(".google.com"); } catch { return true; } }
