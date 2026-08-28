import { homedir } from "node:os";
import { join } from "node:path";
import type { AppConfig } from "./types";

/** Honour XDG so cached pages land beside every other tool's cache. */
function cacheHome(): string {
  const xdg = process.env.XDG_CACHE_HOME;
  return join(xdg && xdg.startsWith("/") ? xdg : join(homedir(), ".cache"), "search-headless");
}

export const DEFAULT_CONFIG: AppConfig = {
  searchLimit: 10,
  fetchLimit: 5,
  timeoutMs: 60_000,
  maxChars: 50_000,
  concurrency: 5,
  cacheDir: process.env.SEARCH_HEADLESS_CACHE_DIR || cacheHome(),
  // A SERP goes stale quickly; an article does not.
  searchCacheTtlMs: 15 * 60_000,
  fetchCacheTtlMs: 6 * 60 * 60_000,
  maxResponseBytes: 8 * 1024 * 1024,
};
