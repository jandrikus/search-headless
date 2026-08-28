import { expect, it } from "bun:test";
import { DEFAULT_CONFIG } from "../../src/config";

it("defines agent-friendly defaults", () => {
  expect(DEFAULT_CONFIG.searchLimit).toBe(10);
  expect(DEFAULT_CONFIG.fetchLimit).toBe(5);
  expect(DEFAULT_CONFIG.timeoutMs).toBe(60_000);
  expect(DEFAULT_CONFIG.maxChars).toBe(50_000);
  expect(DEFAULT_CONFIG.concurrency).toBe(5);
});

it("puts the cache under XDG_CACHE_HOME", () => {
  expect(DEFAULT_CONFIG.cacheDir).toContain("search-headless");
});
