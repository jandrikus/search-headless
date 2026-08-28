#!/usr/bin/env bun
import { Command } from "commander";
import { DEFAULT_CONFIG } from "../config";
import { searchBrave } from "../search/brave";
import { renderJson } from "../output/json";
import { renderSearchMarkdown } from "../output/markdown";
import { emit, exitCodeFor, parseCliOptions, positiveInt } from "./shared";

const program = new Command();

program
  .argument("<query>")
  .option("--limit <n>", "results", String(DEFAULT_CONFIG.searchLimit))
  .option("--fetch")
  .option("--fetch-limit <n>", "fetch limit", String(DEFAULT_CONFIG.fetchLimit))
  .option("--format <format>", "json|markdown", "json")
  .option("--timeout <ms>", "timeout", String(DEFAULT_CONFIG.timeoutMs))
  .option("--concurrency <n>", "parallel fetches", String(DEFAULT_CONFIG.concurrency))
  .option("--no-cache", "ignore cached results and do not store new ones")
  .option("--cache-ttl <s>", "how long a cached result stays fresh, in seconds")
  .action(async (query, opts) => {
    const parsed = parseCliOptions(opts);

    const result = await searchBrave(query, {
      limit: positiveInt(opts.limit, "limit"),
      fetch: Boolean(opts.fetch),
      fetchLimit: positiveInt(opts.fetchLimit, "fetch-limit"),
      timeoutMs: parsed.timeoutMs,
      concurrency: parsed.concurrency,
      cache: { ...parsed.cache, ttlMs: parsed.cache.ttlMs ?? DEFAULT_CONFIG.searchCacheTtlMs },
      allowPrivateUrls: process.env.SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS === "1",
    });

    await emit(opts.format === "markdown" ? renderSearchMarkdown(result) : renderJson(result));
    process.exit(exitCodeFor(result.status));
  });

program.parse();
