import { afterEach, expect, it } from "bun:test";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildObscuraArgs, obscuraFetch, parseObscuraJson, resolveWaitSecs } from "../../src/obscura/runner";
import { createDeadline } from "../../src/types";

const dirs: string[] = [];
const originalBin = process.env.OBSCURA_BIN;
afterEach(async () => {
  while (dirs.length) await rm(dirs.pop()!, { recursive: true, force: true });
  if (originalBin === undefined) delete process.env.OBSCURA_BIN;
  else process.env.OBSCURA_BIN = originalBin;
});

/** A stand-in obscura that records its argv and prints `emit` on stdout. */
async function fakeObscura(emit: string, exitCode = 0) {
  const dir = await mkdtemp(join(tmpdir(), "sh-obscura-"));
  dirs.push(dir);
  const capture = join(dir, "argv.txt");
  const bin = join(dir, "fake-obscura");
  await writeFile(bin, `#!/usr/bin/env bash\nprintf '%s\\n' "$@" > "${capture}"\nprintf '%s' ${JSON.stringify(emit)}\nexit ${exitCode}\n`);
  await chmod(bin, 0o755);
  process.env.OBSCURA_BIN = bin;
  return { capture };
}

it("clamps --wait so a long deadline does not idle the page", () => {
  // The original code passed ceil(timeout/1000), so a 60s deadline meant --wait 60.
  expect(resolveWaitSecs(createDeadline(60_000))).toBeLessThanOrEqual(8);
  expect(resolveWaitSecs(createDeadline(60_000))).toBeGreaterThan(0);
});

it("clamps --wait down to a short remaining deadline", () => {
  expect(resolveWaitSecs(createDeadline(2_000))).toBeLessThanOrEqual(2);
});

it("builds the expected argv", () => {
  const args = buildObscuraArgs({ url: "https://example.com", deadline: createDeadline(30_000), stealth: true, evalScript: "1+1" });
  expect(args[1]).toBe("fetch");
  expect(args[2]).toBe("https://example.com");
  expect(args).toContain("--quiet");
  expect(args).toContain("--stealth");
  expect(args).toContain("--eval");
  expect(args[args.indexOf("--eval") + 1]).toBe("1+1");
  expect(args).toContain("--timeout");
});

it("omits optional flags that were not requested", () => {
  const args = buildObscuraArgs({ url: "https://example.com", deadline: createDeadline(30_000) });
  expect(args).not.toContain("--stealth");
  expect(args).not.toContain("--eval");
  expect(args).not.toContain("--allow-private-network");
});

it("passes --allow-private-network only when asked", () => {
  const args = buildObscuraArgs({ url: "http://127.0.0.1:1", deadline: createDeadline(30_000), allowPrivateNetwork: true });
  expect(args).toContain("--allow-private-network");
});

it("runs the binary and hands back its stdout", async () => {
  const { capture } = await fakeObscura('{"ok":true}');
  const result = await obscuraFetch({ url: "https://example.com", deadline: createDeadline(10_000), stealth: true });

  expect(result.ok).toBe(true);
  expect(result.stdout).toBe('{"ok":true}');
  const argv = (await readFile(capture, "utf-8")).split("\n");
  expect(argv).toContain("--stealth");
  expect(argv).toContain("https://example.com");
});

it("reports a non-zero exit without throwing", async () => {
  await fakeObscura("", 3);
  const result = await obscuraFetch({ url: "https://example.com", deadline: createDeadline(10_000) });
  expect(result.ok).toBe(false);
  expect(result.exitCode).toBe(3);
});

it("reports a missing binary rather than throwing", async () => {
  process.env.OBSCURA_BIN = "/nonexistent/obscura-does-not-exist";
  const result = await obscuraFetch({ url: "https://example.com", deadline: createDeadline(10_000) });
  expect(result.ok).toBe(false);
});

it("treats empty or non-JSON output as no result", () => {
  // `--eval` prints whatever the script returned, so a throwing script exits 0 with junk.
  expect(parseObscuraJson("")).toBeNull();
  expect(parseObscuraJson("not json")).toBeNull();
  expect(parseObscuraJson("null")).toBeNull();
  expect(parseObscuraJson("42")).toBeNull();
  expect(parseObscuraJson<{ a: number }>('{"a":1}')).toEqual({ a: 1 });
});
