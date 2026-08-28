import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { DEFAULT_CONFIG } from "./config";

/**
 * A tiny content-addressed disk cache.
 *
 * An agent asks for the same page across turns and reruns the same query while
 * exploring; every repeat otherwise costs a browser launch and, worse, one more
 * request against Brave's rate limit. Entries are written atomically so a
 * concurrent reader never sees a half-written file, and only successful results
 * are ever stored — a `blocked` SERP cached for fifteen minutes would be a bug,
 * not a speedup.
 */

export interface CacheOptions {
  /** Set false to bypass reads and writes entirely (`--no-cache`). */
  enabled?: boolean;
  /** Freshness window in ms. Falls back to the namespace default. */
  ttlMs?: number;
  /** Override the cache root, mainly for tests. */
  dir?: string;
}

interface Envelope<T> {
  expiresAt: number;
  value: T;
}

/** One in this many writes also sweeps expired entries, to bound growth. */
const PRUNE_ODDS = 20;

function cacheEnabled(options: CacheOptions): boolean {
  if (options.enabled === false) return false;
  return process.env.SEARCH_HEADLESS_NO_CACHE !== "1";
}

function namespaceDir(namespace: string, options: CacheOptions): string {
  return join(options.dir ?? DEFAULT_CONFIG.cacheDir, namespace);
}

function entryPath(namespace: string, key: string, options: CacheOptions): string {
  const hash = createHash("sha256").update(key).digest("hex").slice(0, 32);
  return join(namespaceDir(namespace, options), `${hash}.json`);
}

/** Read a fresh entry, or undefined on a miss, an expiry or a damaged file. */
export function cacheRead<T>(namespace: string, key: string, options: CacheOptions = {}): T | undefined {
  if (!cacheEnabled(options)) return undefined;
  try {
    const envelope = JSON.parse(readFileSync(entryPath(namespace, key, options), "utf-8")) as Envelope<T>;
    if (typeof envelope?.expiresAt !== "number" || envelope.expiresAt <= Date.now()) return undefined;
    return envelope.value;
  } catch {
    return undefined;
  }
}

/** Store a value. Failures are silent: a cache that cannot write is not an error. */
export function cacheWrite<T>(namespace: string, key: string, value: T, options: CacheOptions = {}): void {
  if (!cacheEnabled(options)) return;
  const ttlMs = options.ttlMs ?? DEFAULT_CONFIG.fetchCacheTtlMs;
  if (ttlMs <= 0) return;
  const path = entryPath(namespace, key, options);
  try {
    mkdirSync(namespaceDir(namespace, options), { recursive: true });
    const tmp = `${path}.${process.pid}.tmp`;
    writeFileSync(tmp, JSON.stringify({ expiresAt: Date.now() + ttlMs, value } satisfies Envelope<T>));
    renameSync(tmp, path);
    if (Math.floor(Math.random() * PRUNE_ODDS) === 0) pruneCache(namespace, options);
  } catch {
    // Best effort.
  }
}

/**
 * Delete expired entries in a namespace.
 *
 * Expiry lives inside the file, so this parses each one; the sweep is rare
 * enough (see PRUNE_ODDS) that the cost does not land on a hot path.
 */
export function pruneCache(namespace: string, options: CacheOptions = {}): number {
  const dir = namespaceDir(namespace, options);
  let removed = 0;
  try {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      try {
        if (name.endsWith(".tmp")) {
          // Left behind by a process that died mid-write.
          if (Date.now() - statSync(path).mtimeMs > 60_000) { rmSync(path, { force: true }); removed++; }
          continue;
        }
        const envelope = JSON.parse(readFileSync(path, "utf-8")) as Envelope<unknown>;
        if (typeof envelope?.expiresAt !== "number" || envelope.expiresAt <= Date.now()) { rmSync(path, { force: true }); removed++; }
      } catch {
        rmSync(path, { force: true });
        removed++;
      }
    }
  } catch {
    // No directory yet, nothing to prune.
  }
  return removed;
}

/** Drop every entry in a namespace, or the whole cache when none is named. */
export function clearCache(namespace?: string, options: CacheOptions = {}): void {
  const target = namespace ? namespaceDir(namespace, options) : (options.dir ?? DEFAULT_CONFIG.cacheDir);
  rmSync(target, { recursive: true, force: true });
}
