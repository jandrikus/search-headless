import { closeSync, mkdirSync, openSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import type { Deadline } from "./types";
import { remainingMs } from "./types";

/**
 * Cross-process spacing for Brave searches.
 *
 * The timestamp alone is not enough: two `search-web` processes started together
 * both read the same old timestamp, both compute the same wait, and both fire at
 * the same instant — exactly the burst the spacing exists to prevent. So the
 * whole read-wait-stamp sequence runs under an exclusive lock, which makes the
 * second process queue behind the first rather than race it.
 */

/** Minimum seconds between searches to appear human-like. */
export const MIN_SEARCH_INTERVAL_SEC = 10;

/** Timestamp of the last search. In /tmp so it costs no disk I/O. */
export const LAST_SEARCH_FILE = process.env.SEARCH_HEADLESS_RATE_FILE || "/tmp/search-headless-last-search";

const LOCK_FILE = `${LAST_SEARCH_FILE}.lock`;

/** A lock older than this belonged to a process that died holding it. */
const STALE_LOCK_MS = MIN_SEARCH_INTERVAL_SEC * 3 * 1000;

export interface RateLimitOptions {
  deadline: Deadline;
  intervalSec?: number;
  file?: string;
}

export type RateLimitOutcome =
  /** A slot was taken; the search may run. */
  | { ok: true; waitedMs: number }
  /** The wait would outlast the caller's deadline; nothing was reserved. */
  | { ok: false; requiredMs: number };

function disabled(): boolean {
  return process.env.SEARCH_HEADLESS_NO_RATE_LIMIT === "1";
}

/** Take the lock, or return false if it is held by a live process. */
function tryLock(path: string): boolean {
  try {
    closeSync(openSync(path, "wx"));
    return true;
  } catch {
    try {
      if (Date.now() - statSync(path).mtimeMs > STALE_LOCK_MS) {
        rmSync(path, { force: true });
        closeSync(openSync(path, "wx"));
        return true;
      }
    } catch {
      // Someone else won the race to break the stale lock.
    }
    return false;
  }
}

/**
 * Wait until the next search slot is free, then claim it.
 *
 * Returns `ok: false` rather than overrunning the caller's deadline, so a short
 * `--timeout` reports a timeout instead of silently blocking for ten seconds.
 */
export async function reserveSearchSlot(options: RateLimitOptions): Promise<RateLimitOutcome> {
  if (disabled()) return { ok: true, waitedMs: 0 };

  const file = options.file ?? LAST_SEARCH_FILE;
  const lock = `${file}.lock`;
  const intervalMs = (options.intervalSec ?? MIN_SEARCH_INTERVAL_SEC) * 1000;
  const startedAt = Date.now();

  try {
    mkdirSync(dirname(file), { recursive: true });
  } catch {
    // Directory already exists, or is not ours to create.
  }

  // Queue for the lock. Another process holding it is itself waiting out the
  // interval, so its wait counts against ours and is checked against the deadline.
  while (!tryLock(lock)) {
    if (remainingMs(options.deadline) <= 250) {
      return { ok: false, requiredMs: intervalMs };
    }
    await Bun.sleep(100);
  }

  try {
    let waitMs = 0;
    try {
      const last = Number.parseInt(readFileSync(file, "utf-8").trim(), 10);
      if (Number.isFinite(last)) waitMs = Math.max(0, intervalMs - (Date.now() - last));
    } catch {
      // No previous search recorded.
    }

    if (waitMs >= remainingMs(options.deadline)) {
      return { ok: false, requiredMs: waitMs };
    }
    if (waitMs > 0) await Bun.sleep(waitMs);

    try {
      writeFileSync(file, String(Date.now()));
    } catch {
      // Spacing is best effort; a read-only /tmp must not fail the search.
    }
    return { ok: true, waitedMs: Date.now() - startedAt };
  } finally {
    rmSync(lock, { force: true });
  }
}
