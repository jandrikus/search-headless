#!/usr/bin/env bun
import { Command } from "commander";
import { DEFAULT_CONFIG } from "../config";
import { fetchContent, fetchContentBatch } from "../fetch/fetch-content";
import { renderJson } from "../output/json";
import { renderFetchedMarkdown, renderFetchBatchMarkdown } from "../output/markdown";
import { emit, exitCodeFor, parseCliOptions } from "./shared";

const program = new Command();

program
  .argument("<url...>", "one or more URLs; several are fetched in parallel")
  .option("--format <format>", "json|markdown", "json")
  .option("--timeout <ms>", "timeout", String(DEFAULT_CONFIG.timeoutMs))
  .option("--max-chars <n>", "max chars", String(DEFAULT_CONFIG.maxChars))
  .option("--concurrency <n>", "parallel fetches", String(DEFAULT_CONFIG.concurrency))
  .option("--no-cache", "ignore cached results and do not store new ones")
  .option("--cache-ttl <s>", "how long a cached result stays fresh, in seconds")
  .action(async (urls: string[], opts) => {
    const parsed = parseCliOptions(opts);
    const options = {
      timeoutMs: parsed.timeoutMs,
      maxChars: parsed.maxChars,
      concurrency: parsed.concurrency,
      cache: parsed.cache,
      allowPrivateUrls: process.env.SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS === "1",
    };

    // One URL keeps the single-object shape callers already parse; several get a
    // batch envelope so a per-URL failure stays attached to its URL.
    if (urls.length === 1) {
      const result = await fetchContent(urls[0], options);
      await emit(opts.format === "markdown" ? renderFetchedMarkdown(result) : renderJson(result));
      process.exit(exitCodeFor(result.status));
    }

    const batch = await fetchContentBatch(urls, options);
    await emit(opts.format === "markdown" ? renderFetchBatchMarkdown(batch) : renderJson(batch));
    process.exit(exitCodeFor(batch.status));
  });

program.parse();
