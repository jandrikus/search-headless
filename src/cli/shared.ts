import type { CacheOptions } from "../cache";
import { DEFAULT_CONFIG } from "../config";
import type { CommandStatus } from "../types";

/** Argument errors exit 2, so a bad flag is never mistaken for a failed search. */
export function fail(message: string): never {
  console.error(message);
  process.exit(2);
}

export function positiveInt(value: string, name: string): number {
  const n = Number(value);
  if (!Number.isInteger(n) || n <= 0) fail(`${name} must be a positive integer`);
  return n;
}

export interface ParsedCliOptions {
  timeoutMs: number;
  maxChars: number;
  concurrency: number;
  cache: CacheOptions;
}

/** Validate the flags every command shares. */
export function parseCliOptions(opts: Record<string, string | boolean | undefined>): ParsedCliOptions {
  if (typeof opts.format === "string" && !["json", "markdown"].includes(opts.format)) fail("Invalid format");

  return {
    timeoutMs: positiveInt(String(opts.timeout ?? DEFAULT_CONFIG.timeoutMs), "timeout"),
    maxChars: positiveInt(String(opts.maxChars ?? DEFAULT_CONFIG.maxChars), "max-chars"),
    concurrency: positiveInt(String(opts.concurrency ?? DEFAULT_CONFIG.concurrency), "concurrency"),
    cache: {
      // commander turns `--no-cache` into `cache: false`.
      enabled: opts.cache !== false,
      ttlMs: opts.cacheTtl === undefined ? undefined : positiveInt(String(opts.cacheTtl), "cache-ttl") * 1000,
    },
  };
}

/**
 * Write the result and wait for it to land.
 *
 * `process.exit` right after `process.stdout.write` can drop the tail of a large
 * payload when stdout is a pipe, which is exactly how an agent calls this.
 */
export async function emit(text: string): Promise<void> {
  await Bun.write(Bun.stdout, text);
}

/** `partial` still carries usable results, so it exits 0. */
export function exitCodeFor(status: CommandStatus): number {
  return status === "ok" || status === "partial" ? 0 : 1;
}
