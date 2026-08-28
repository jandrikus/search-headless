import { afterEach, expect, it } from "bun:test";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { JSDOM } from "jsdom";
import { READ_SERP_SCRIPT, searchBrave } from "../../src/search/brave";
import { parseBraveNextUrl, parseBraveResults } from "../../src/search/parse-brave";

const dirs: string[] = [];
const originalBin = process.env.OBSCURA_BIN;
afterEach(async () => {
  while (dirs.length) await rm(dirs.pop()!, { recursive: true, force: true });
  if (originalBin === undefined) delete process.env.OBSCURA_BIN;
  else process.env.OBSCURA_BIN = originalBin;
});

/**
 * Stand-in obscura that answers with real SERP HTML, so searchBrave's spawn,
 * parse and pagination path is exercised without a browser. A `--limit`-driven
 * second page is served for `offset=` URLs; `challenge` in the query yields the
 * blocked fixture.
 */
async function fakeObscura() {
  const dir = await mkdtemp(join(tmpdir(), "sh-search-"));
  dirs.push(dir);
  const bin = join(dir, "fake-obscura");
  // argv: fetch <url> --quiet ...; the URL decides which fixture to emit.
  await writeFile(
    bin,
    `#!/usr/bin/env bash
url="$2"
if [[ "$url" == *"challenge"* ]]; then page=tests/fixtures/brave-blocked.html
elif [[ "$url" == *"offset="* ]]; then page=tests/fixtures/brave-results-page2.html
else page=tests/fixtures/brave-results.html
fi
html=$(cat "$page")
python3 -c "
import json,sys
html=sys.stdin.read()
print(json.dumps({'html':html,'text':html,'url':'https://search.brave.com/search'}))
" <<<"$html"
`,
  );
  await chmod(bin, 0o755);
  process.env.OBSCURA_BIN = bin;
}

process.env.SEARCH_HEADLESS_NO_RATE_LIMIT = "1";
const noRateLimit = { timeoutMs: 20_000 };

it("returns parsed results through the real obscura spawn path", async () => {
  await fakeObscura();
  const response = await searchBrave("example", { limit: 2, ...noRateLimit });

  expect(response.status).toBe("ok");
  expect(response.results).toHaveLength(2);
  expect(response.results[0].title).toBe("First Test Article - Example");
});

it("paginates through the spawn path and ranks across pages", async () => {
  await fakeObscura();
  const response = await searchBrave("example", { limit: 5, ...noRateLimit });

  expect(response.results).toHaveLength(5);
  expect(response.results[4].title).toBe("Fifth Result - Other Site");
  expect(response.results[4].rank).toBe(5);
});

it("reports a challenge from the spawn path as blocked", async () => {
  await fakeObscura();
  const response = await searchBrave("challenge please", { limit: 5, ...noRateLimit });

  expect(response.status).toBe("blocked");
  expect(response.results).toHaveLength(0);
  expect(response.errors[0].code).toBe("BRAVE_CHALLENGE");
});

/**
 * A browser that will not start is not a bot challenge. Reporting it as one
 * would send an agent off to wait out a block that does not exist.
 */
it("reports a broken obscura as an error, not as blocked", async () => {
  process.env.OBSCURA_BIN = "/nonexistent/obscura";
  const response = await searchBrave("anything", { limit: 3, ...noRateLimit, cache: { enabled: false } });

  expect(response.status).toBe("error");
  expect(response.errors[0].code).toBe("OBSCURA_SEARCH_FAILED");
  expect(response.results).toHaveLength(0);
});

/**
 * The eval script runs inside obscura, so nothing above would notice if it
 * referenced a property that does not exist. Run it against a real DOM.
 */
it("READ_SERP_SCRIPT returns html, text and url from a real document", async () => {
  const html = await Bun.file("tests/fixtures/brave-results.html").text();
  const dom = new JSDOM(html, { url: "https://search.brave.com/search?q=test", runScripts: "outside-only" });

  const raw = dom.window.eval(READ_SERP_SCRIPT) as string;
  const payload = JSON.parse(raw) as { html: string; text: string; url: string };

  expect(payload.url).toContain("search.brave.com");
  expect(payload.text.length).toBeGreaterThan(0);
  // The captured HTML must still be parseable, including the footer "Next" link.
  expect(parseBraveResults(payload.html, 10)).toHaveLength(3);
  expect(parseBraveNextUrl(payload.html)).toContain("offset=");
});
