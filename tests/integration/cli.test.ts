import { afterEach, expect, it } from "bun:test";
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
  const proc = await run(["src/cli/search-web.ts", "example", "--limit", "2"], { SEARCH_HEADLESS_BRAVE_FIXTURE: "tests/fixtures/brave-results.html" });
  expect(proc.exitCode).toBe(0); const json = JSON.parse(proc.stdout); expect(json.engine).toBe("brave"); expect(json.results).toHaveLength(2); expect(json.results[0].title).toBe("First Test Article - Example");
});

it("search-web fixture mode paginates when more results are requested", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--limit", "5"], { SEARCH_HEADLESS_BRAVE_FIXTURE: "tests/fixtures/brave-results.html", SEARCH_HEADLESS_BRAVE_FIXTURE_2: "tests/fixtures/brave-results-page2.html" });
  expect(proc.exitCode).toBe(0); const json = JSON.parse(proc.stdout); expect(json.results).toHaveLength(5); expect(json.results[4].title).toBe("Fifth Result - Other Site"); expect(json.results[4].rank).toBe(5);
});

it("search-web reports a challenge as blocked, never as an empty success", async () => {
  const proc = await run(["src/cli/search-web.ts", "example"], { SEARCH_HEADLESS_BRAVE_FIXTURE: "tests/fixtures/brave-blocked.html" });
  expect(proc.exitCode).not.toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("blocked"); expect(json.results).toHaveLength(0); expect(json.errors[0].code).toBe("BRAVE_CHALLENGE");
});

it("search-web keeps a later-page challenge terminal, exiting non-zero with earlier results", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--limit", "5"], { SEARCH_HEADLESS_BRAVE_FIXTURE: "tests/fixtures/brave-results.html", SEARCH_HEADLESS_BRAVE_FIXTURE_2: "tests/fixtures/brave-blocked.html" });
  expect(proc.exitCode).not.toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("blocked"); expect(json.results).toHaveLength(3); expect(json.errors[0].code).toBe("BRAVE_CHALLENGE");
});

it("search-web rejects the removed Google flag", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--google"]);
  expect(proc.exitCode).not.toBe(0); expect(proc.stderr).toContain("unknown option");
});

it("fetch-content exhausted timeout returns timeout status", async () => {
  const proc = await run(["src/cli/fetch-content.ts", "https://example.com", "--timeout", "1"]);
  expect(proc.exitCode).not.toBe(0); const json = JSON.parse(proc.stdout); expect(json.status).toBe("timeout");
});

it("fetch-content browser fallback timeout returns structured JSON", async () => {
  // With a 1ms timeout every tier declines to start.
  const proc = await run(["src/cli/fetch-content.ts", "https://example.com", "--timeout", "1"]);
  expect(proc.exitCode).not.toBe(0); expect(proc.stdout).not.toBe(""); const json = JSON.parse(proc.stdout); expect(["timeout", "error"]).toContain(json.status); expect(json.errors[0].code).toMatch(/FETCH_TIMEOUT/);
});

it("fetch-content fetches several urls in one run and keeps them in order", async () => {
  const html = (name: string) => `<!doctype html><html><head><title>${name}</title></head><body><article><p>${"Body of " + name + ". "}${"filler ".repeat(80)}</p></article></body></html>`;
  const base = server((req) => new Response(html(new URL(req.url).pathname.slice(1)), { headers: { "content-type": "text/html" } }));
  const proc = await run(["src/cli/fetch-content.ts", `${base}/alpha`, `${base}/beta`], { SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS: "1" });

  expect(proc.exitCode).toBe(0);
  const json = JSON.parse(proc.stdout);
  expect(json.status).toBe("ok");
  expect(json.results.map((r: { title: string }) => r.title)).toEqual(["alpha", "beta"]);
});

it("fetch-content reports a batch where only some urls worked as partial, exiting zero", async () => {
  const base = server((req) => new URL(req.url).pathname === "/good"
    ? new Response(`<!doctype html><html><head><title>Good</title></head><body><article><p>${"real content ".repeat(60)}</p></article></body></html>`, { headers: { "content-type": "text/html" } })
    : new Response("nope", { status: 404, headers: { "content-type": "text/html" } }));
  const proc = await run(["src/cli/fetch-content.ts", `${base}/good`, `${base}/bad`], { SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS: "1" });

  expect(proc.exitCode).toBe(0);
  const json = JSON.parse(proc.stdout);
  expect(json.status).toBe("partial");
  expect(json.results[0].status).toBe("ok");
  expect(json.results[1].status).toBe("error");
});

it("fetch-content --no-cache re-requests the page", async () => {
  let hits = 0;
  const base = server(() => { hits++; return new Response(`<!doctype html><html><head><title>Counted</title></head><body><article><p>${"content ".repeat(80)}</p></article></body></html>`, { headers: { "content-type": "text/html" } }); });
  const env = { SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS: "1" };

  await run(["src/cli/fetch-content.ts", `${base}/page`], env);
  await run(["src/cli/fetch-content.ts", `${base}/page`], env);
  expect(hits).toBe(1);

  await run(["src/cli/fetch-content.ts", `${base}/page`, "--no-cache"], env);
  expect(hits).toBe(2);
});

it("rejects a nonsense flag value before doing any work", async () => {
  const proc = await run(["src/cli/search-web.ts", "example", "--limit", "0"]);
  expect(proc.exitCode).toBe(2);
  expect(proc.stderr).toContain("limit must be a positive integer");
});
