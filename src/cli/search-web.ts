#!/usr/bin/env bun
import { Command } from "commander";
import { DEFAULT_CONFIG } from "../config";
import { searchBrave } from "../search/brave";
import { searchGoogle } from "../search/google";
import { renderJson } from "../output/json";
import { renderSearchMarkdown } from "../output/markdown";

function positiveInt(value: string, name: string): number {
  const n = Number(value);
  if (!Number.isInteger(n) || n <= 0) throw new Error(`${name} must be a positive integer`);
  return n;
}

const program = new Command();

program
  .argument("<query>")
  .option("--limit <n>", "results", String(DEFAULT_CONFIG.searchLimit))
  .option("--fetch")
  .option("--fetch-limit <n>", "fetch limit", String(DEFAULT_CONFIG.fetchLimit))
  .option("--format <format>", "json|markdown", "json")
  .option("--google", "use Google instead of Brave (default)")
  .option("--headful", "show browser (Playwright only)")
  .option("--no-human", "fail fast on CAPTCHA (Google only)")
  .option("--timeout <ms>", "timeout", String(DEFAULT_CONFIG.timeoutMs))
  .action(async (query, opts) => {
    if (!["json", "markdown"].includes(opts.format)) {
      console.error("Invalid format");
      process.exit(2);
    }

    let limit: number;
    let fetchLimit: number;
    let timeoutMs: number;

    try {
      limit = positiveInt(opts.limit, "limit");
      fetchLimit = positiveInt(opts.fetchLimit, "fetch-limit");
      timeoutMs = positiveInt(opts.timeout, "timeout");
    } catch (e) {
      console.error(e instanceof Error ? e.message : String(e));
      process.exit(2);
    }

    const searchOptions = {
      limit,
      fetch: Boolean(opts.fetch),
      fetchLimit,
      timeoutMs,
      allowPrivateUrls: process.env.SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS === "1",
    };

    // Use Google if --google flag is set, otherwise default to Brave
    const result = opts.google
      ? await searchGoogle(query, {
          ...searchOptions,
          noHuman: opts.human === false,
          headful: Boolean(opts.headful),
        })
      : await searchBrave(query, searchOptions);

    process.stdout.write(
      opts.format === "markdown" ? renderSearchMarkdown(result) : renderJson(result),
    );
    process.exit(result.status === "ok" || result.status === "partial" ? 0 : 1);
  });

program.parse();
