import { afterEach, expect, it } from "vitest";
const servers: ReturnType<typeof Bun.serve>[] = [];
afterEach(() => { while (servers.length) servers.pop()!.stop(true); });
function server(handler: (req: Request) => Response) { const s = Bun.serve({ port: 0, fetch: handler }); servers.push(s); return `http://127.0.0.1:${s.port}`; }
async function run(args: string[], env: Record<string,string> = {}) {
  const proc = Bun.spawn({ cmd: ["bun", "run", ...args], env: { ...process.env, ...env }, stdout: "pipe", stderr: "pipe" });
  const [exitCode, stdout, stderr] = await Promise.all([proc.exited, new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { exitCode, stdout, stderr };
}

it("fetch-content rejects private urls by default with JSON", async () => {
  const proc = await run(["src/cli/fetch-content.ts", "http://127.0.0.1:1"]);
  expect(proc.exitCode).not.toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("error"); expect(json.errors[0].code).toBe("PRIVATE_URL_BLOCKED");
});

it("fetch-content extracts local fixture only in test mode", async () => {
  const html = await Bun.file("tests/fixtures/article.html").text(); const base = server(() => new Response(html, { headers: { "content-type": "text/html" } }));
  const proc = await run(["src/cli/fetch-content.ts", `${base}/a`], { SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS: "1" });
  expect(proc.exitCode).toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("ok"); expect(json.text).toContain("useful paragraph");
});

it("search-web fixture mode returns parsed JSON", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--limit", "2", "--no-human"], { SEARCH_HEADLESS_GOOGLE_FIXTURE: "tests/fixtures/google-results.html" });
  expect(proc.exitCode).toBe(0); const json = JSON.parse(proc.stdout); expect(json.results).toHaveLength(2); expect(json.results[0].title).toBe("Alpha");
});

it("search-web fixture mode paginates when more results are requested", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--limit", "4", "--no-human"], { SEARCH_HEADLESS_GOOGLE_FIXTURE: "tests/fixtures/google-results.html", SEARCH_HEADLESS_GOOGLE_FIXTURE_2: "tests/fixtures/google-results-page2.html" });
  expect(proc.exitCode).toBe(0); const json = JSON.parse(proc.stdout); expect(json.results.map((r: any) => r.title)).toEqual(["Alpha", "Beta", "Gamma", "Delta"]); expect(json.results[3].rank).toBe(4);
});

it("search-web blocked fixture exits non-zero with structured JSON", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--no-human"], { SEARCH_HEADLESS_GOOGLE_FIXTURE: "tests/fixtures/google-blocked.html" });
  expect(proc.exitCode).not.toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("blocked"); expect(json.errors[0].code).toBe("HUMAN_INTERVENTION_REQUIRED");
});

it("search-web later-page block with no-human exits non-zero as blocked", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--limit", "4", "--no-human"], { SEARCH_HEADLESS_GOOGLE_FIXTURE: "tests/fixtures/google-results.html", SEARCH_HEADLESS_GOOGLE_FIXTURE_2: "tests/fixtures/google-blocked.html" });
  expect(proc.exitCode).not.toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("blocked"); expect(json.results).toHaveLength(2); expect(json.errors[0].code).toBe("HUMAN_INTERVENTION_REQUIRED");
});

it("fetch-content exhausted timeout returns timeout status", async () => {
  const proc = await run(["src/cli/fetch-content.ts", "https://example.com", "--timeout", "1"]);
  expect(proc.exitCode).not.toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("timeout");
});

it("fetch-content browser fallback timeout returns structured JSON", async () => {
  // With a 1ms timeout, HTTP extraction times out (recoverable) and obscura also times out.
  const proc = await run(["src/cli/fetch-content.ts", "https://example.com", "--timeout", "1"]);
  expect(proc.exitCode).not.toBe(0); expect(proc.stdout).not.toBe(""); const json = JSON.parse(proc.stdout); expect(["timeout", "error"]).toContain(json.status); expect(json.errors[0].code).toMatch(/FETCH_TIMEOUT/);
});
