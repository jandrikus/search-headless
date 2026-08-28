import { afterEach, beforeEach, expect, it } from "bun:test";
import { mkdtempSync } from "node:fs";
import { rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { reserveSearchSlot } from "../../src/rate-limit";
import { createDeadline } from "../../src/types";

const dirs: string[] = [];
let file: string;
const originalDisable = process.env.SEARCH_HEADLESS_NO_RATE_LIMIT;

beforeEach(() => {
  const dir = mkdtempSync(join(tmpdir(), "sh-rate-"));
  dirs.push(dir);
  file = join(dir, "last-search");
  delete process.env.SEARCH_HEADLESS_NO_RATE_LIMIT;
});

afterEach(async () => {
  if (originalDisable === undefined) delete process.env.SEARCH_HEADLESS_NO_RATE_LIMIT;
  else process.env.SEARCH_HEADLESS_NO_RATE_LIMIT = originalDisable;
  while (dirs.length) await rm(dirs.pop()!, { recursive: true, force: true });
});

it("lets the first search through immediately", async () => {
  const outcome = await reserveSearchSlot({ deadline: createDeadline(5000), intervalSec: 1, file });
  expect(outcome.ok).toBe(true);
});

it("spaces a second search by the interval", async () => {
  await reserveSearchSlot({ deadline: createDeadline(5000), intervalSec: 1, file });
  const started = Date.now();
  await reserveSearchSlot({ deadline: createDeadline(5000), intervalSec: 1, file });
  expect(Date.now() - started).toBeGreaterThanOrEqual(900);
});

/**
 * The bug this guards: both callers read the same old timestamp, compute the
 * same wait and then fire together, which is the burst the spacing exists to
 * prevent. They have to queue, not race.
 */
it("serializes concurrent callers instead of releasing them together", async () => {
  await reserveSearchSlot({ deadline: createDeadline(10_000), intervalSec: 1, file });
  const started = Date.now();

  const finishes = await Promise.all(
    [0, 1].map(async () => {
      await reserveSearchSlot({ deadline: createDeadline(10_000), intervalSec: 1, file });
      return Date.now() - started;
    }),
  );

  finishes.sort((a, b) => a - b);
  expect(finishes[0]).toBeGreaterThanOrEqual(900);
  expect(finishes[1] - finishes[0]).toBeGreaterThanOrEqual(900);
});

it("declines rather than overrunning the caller's deadline", async () => {
  await reserveSearchSlot({ deadline: createDeadline(5000), intervalSec: 10, file });
  const outcome = await reserveSearchSlot({ deadline: createDeadline(300), intervalSec: 10, file });
  expect(outcome.ok).toBe(false);
});

it("is a no-op when disabled", async () => {
  process.env.SEARCH_HEADLESS_NO_RATE_LIMIT = "1";
  await reserveSearchSlot({ deadline: createDeadline(5000), intervalSec: 30, file });
  const started = Date.now();
  await reserveSearchSlot({ deadline: createDeadline(5000), intervalSec: 30, file });
  expect(Date.now() - started).toBeLessThan(200);
});
