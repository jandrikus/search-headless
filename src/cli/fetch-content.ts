#!/usr/bin/env bun
import { Command } from "commander";
import { DEFAULT_CONFIG } from "../config";
import { fetchContent } from "../fetch/fetch-content";
import { renderJson } from "../output/json";
import { renderFetchedMarkdown } from "../output/markdown";
function positiveInt(value: string, name: string): number { const n = Number(value); if (!Number.isInteger(n) || n <= 0) throw new Error(`${name} must be a positive integer`); return n; }
const program = new Command();
program.argument("<url>").option("--format <format>", "json|markdown", "json").option("--browser").option("--no-browser").option("--headful").option("--no-human").option("--timeout <ms>", "timeout", String(DEFAULT_CONFIG.timeoutMs)).option("--max-chars <n>", "max chars", String(DEFAULT_CONFIG.maxChars)).action(async (url, opts) => {
  if (!["json", "markdown"].includes(opts.format)) { console.error("Invalid format"); process.exit(2); }
  let timeoutMs: number; let maxChars: number;
  try { timeoutMs = positiveInt(opts.timeout, "timeout"); maxChars = positiveInt(opts.maxChars, "max-chars"); } catch (e) { console.error(e instanceof Error ? e.message : String(e)); process.exit(2); }
  const result = await fetchContent(url, { timeoutMs, maxChars, forceBrowser: Boolean(opts.browser), noBrowser: opts.browser === false, headful: Boolean(opts.headful), allowPrivateUrls: process.env.SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS === "1" });
  process.stdout.write(opts.format === "markdown" ? renderFetchedMarkdown(result) : renderJson(result));
  process.exit(result.status === "ok" || result.status === "partial" ? 0 : 1);
});
program.parse();
