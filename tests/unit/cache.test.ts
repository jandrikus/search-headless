import { afterEach, expect, it } from "bun:test";
import { mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cacheRead, cacheWrite, clearCache, pruneCache } from "../../src/cache";

const dirs: string[] = [];
afterEach(async () => { while (dirs.length) await rm(dirs.pop()!, { recursive: true, force: true }); });

function sandbox() {
  const dir = mkdtempSync(join(tmpdir(), "sh-cache-"));
  dirs.push(dir);
  return { dir, ttlMs: 60_000 };
}

it("round-trips a value", () => {
  const options = sandbox();
  cacheWrite("ns", "key", { hello: "world" }, options);
  expect(cacheRead<{ hello: string }>("ns", "key", options)).toEqual({ hello: "world" });
});

it("misses on an unknown key and on another namespace", () => {
  const options = sandbox();
  cacheWrite("ns", "key", 1, options);
  expect(cacheRead("ns", "other", options)).toBeUndefined();
  expect(cacheRead("other", "key", options)).toBeUndefined();
});

it("treats an expired entry as a miss", () => {
  const options = { ...sandbox(), ttlMs: 1 };
  cacheWrite("ns", "key", "stale", options);
  Bun.sleepSync(5);
  expect(cacheRead("ns", "key", options)).toBeUndefined();
});

it("bypasses the cache entirely when disabled", () => {
  const options = sandbox();
  cacheWrite("ns", "key", "value", { ...options, enabled: false });
  expect(cacheRead("ns", "key", options)).toBeUndefined();
  cacheWrite("ns", "key", "value", options);
  expect(cacheRead("ns", "key", { ...options, enabled: false })).toBeUndefined();
});

it("survives a corrupt entry instead of throwing", () => {
  const options = sandbox();
  cacheWrite("ns", "key", "value", options);
  const dir = join(options.dir, "ns");
  writeFileSync(join(dir, readdirSync(dir)[0]), "{ not json");
  expect(cacheRead("ns", "key", options)).toBeUndefined();
});

it("prunes expired and corrupt entries but keeps fresh ones", () => {
  const options = sandbox();
  cacheWrite("ns", "fresh", "keep", options);
  cacheWrite("ns", "stale", "drop", { ...options, ttlMs: 1 });
  Bun.sleepSync(5);

  expect(pruneCache("ns", options)).toBe(1);
  expect(cacheRead<string>("ns", "fresh", options)).toBe("keep");
});

it("clears a namespace", () => {
  const options = sandbox();
  cacheWrite("ns", "key", "value", options);
  clearCache("ns", options);
  expect(cacheRead("ns", "key", options)).toBeUndefined();
});
