import { afterEach, expect, it } from "bun:test";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fetchContent, fetchManyContent } from "../../src/fetch/fetch-content";
import { createDeadline } from "../../src/types";

const servers: ReturnType<typeof Bun.serve>[] = [];
const dirs: string[] = [];
const originalBin = process.env.OBSCURA_BIN;
afterEach(async () => {
  while (servers.length) servers.pop()!.stop(true);
  while (dirs.length) await rm(dirs.pop()!, { recursive: true, force: true });
  if (originalBin === undefined) delete process.env.OBSCURA_BIN;
  else process.env.OBSCURA_BIN = originalBin;
});

function serve(html: string) {
  const s = Bun.serve({ port: 0, fetch: () => new Response(html, { headers: { "content-type": "text/html" } }) });
  servers.push(s);
  return `http://127.0.0.1:${s.port}/page`;
}

/** Stand-in obscura returning the text a real render would have produced. */
async function fakeObscuraReturning(text: string) {
  const dir = await mkdtemp(join(tmpdir(), "sh-tiers-"));
  dirs.push(dir);
  const bin = join(dir, "fake-obscura");
  const payload = JSON.stringify({ title: "Rendered", url: "https://rendered.test/", text });
  await writeFile(bin, `#!/usr/bin/env bash\nprintf '%s' ${JSON.stringify(payload)}\n`);
  await chmod(bin, 0o755);
  process.env.OBSCURA_BIN = bin;
}

const opts = { deadline: createDeadline(20_000), maxChars: 50_000, allowPrivateUrls: true, cache: { enabled: false } };

it("serves a script-free page straight from HTTP without starting a browser", async () => {
  // No fake binary is installed, so reaching obscura at all would fail the test.
  process.env.OBSCURA_BIN = "/nonexistent/obscura";
  const url = serve(`<!doctype html><html><head><title>Short</title></head><body><h1>Short</h1><p>A brief but complete page.</p></body></html>`);

  const result = await fetchContent(url, opts);
  expect(result.status).toBe("ok");
  expect(result.method).toBe("http");
});

it("escalates a JavaScript shell to the browser instead of returning the placeholder", async () => {
  // Regression: HTTP-first previously returned the nav + "Loading…" + cookie
  // banner of an unrendered SPA as a successful extraction.
  await fakeObscuraReturning("The actual rendered article body that only exists after scripts run. ".repeat(12));
  const url = serve(`<!doctype html><html><head><title>Dashboard</title></head><body>
    <nav>Home About Contact</nav>
    <div id="root"><p>Loading your dashboard, please wait...</p></div>
    <div class="cookie">We use cookies to improve your experience. Accept all cookies to continue.</div>
    <script>document.getElementById('root').innerHTML = '<article>real</article>';</script>
  </body></html>`);

  const result = await fetchContent(url, opts);
  expect(result.status).toBe("ok");
  expect(result.method).toBe("browser");
  expect(result.text).toContain("actual rendered article body");
  expect(result.text).not.toContain("Loading your dashboard");
});

it("keeps the HTTP result when the browser cannot improve on it", async () => {
  await fakeObscuraReturning("tiny");
  const url = serve(`<!doctype html><html><head><title>Shell</title></head><body>
    <p>Some short text on a scripted page.</p><script>void 0;</script></body></html>`);

  const result = await fetchContent(url, opts);
  expect(result.status).toBe("ok");
  expect(result.method).toBe("http");
});

it("accepts a long HTTP result on a scripted page without escalating", async () => {
  process.env.OBSCURA_BIN = "/nonexistent/obscura";
  const body = "This is a full article paragraph with plenty of real content. ".repeat(20);
  const url = serve(`<!doctype html><html><head><title>Article</title></head><body><article><h1>Article</h1><p>${body}</p></article><script>void 0;</script></body></html>`);

  const result = await fetchContent(url, opts);
  expect(result.status).toBe("ok");
  expect(result.method).toBe("http");
});

it("fetches many urls concurrently, preserving order", async () => {
  process.env.OBSCURA_BIN = "/nonexistent/obscura";
  const urls = ["Alpha", "Beta", "Gamma"].map((name) =>
    serve(`<!doctype html><html><head><title>${name}</title></head><body><h1>${name}</h1><p>Body of ${name}.</p></body></html>`),
  );

  const results = await fetchManyContent(urls, { ...opts, concurrency: 3 });
  expect(results.map((r) => r.status)).toEqual(["ok", "ok", "ok"]);
  expect(results.map((r) => r.title)).toEqual(["Alpha", "Beta", "Gamma"]);
});

it("keeps a usable HTTP result when the browser tier fails outright", async () => {
  // Regression: a thin-but-real page was discarded entirely when the escalation
  // it triggered could not run, turning content into an error.
  process.env.OBSCURA_BIN = "/nonexistent/obscura";
  const url = serve(`<!doctype html><html><head><title>Shell</title></head><body><p>Loading...</p><script>void 0;</script></body></html>`);

  const result = await fetchContent(url, opts);
  expect(result.status).toBe("ok");
  expect(result.text).toContain("Loading...");
});

it("does not start a browser for a content type rendering cannot fix", async () => {
  process.env.OBSCURA_BIN = "/nonexistent/obscura";
  const s = Bun.serve({ port: 0, fetch: () => new Response("%PDF-1.7", { headers: { "content-type": "application/pdf" } }) });
  servers.push(s);

  const result = await fetchContent(`http://127.0.0.1:${s.port}/doc.pdf`, opts);
  expect(result.status).toBe("error");
  expect(result.errors[0].code).toBe("FETCH_NOT_HTML");
  expect(result.errors).toHaveLength(1); // no browser attempt was appended
});

it("serves a repeat fetch from cache without touching the network", async () => {
  process.env.OBSCURA_BIN = "/nonexistent/obscura";
  let hits = 0;
  const s = Bun.serve({ port: 0, fetch: () => { hits++; return new Response(`<!doctype html><html><head><title>Cached</title></head><body><p>Stable content.</p></body></html>`, { headers: { "content-type": "text/html" } }); } });
  servers.push(s);
  const url = `http://127.0.0.1:${s.port}/page`;
  const cache = { enabled: true, ttlMs: 60_000 };

  const first = await fetchContent(url, { ...opts, cache });
  const second = await fetchContent(url, { ...opts, cache });

  expect(hits).toBe(1);
  expect(first.cached).toBeUndefined();
  expect(second.cached).toBe(true);
  expect(second.text).toBe(first.text!);
});

it("does not render a 404 page and pass its text off as the article", async () => {
  await fakeObscuraReturning("Sorry, that page could not be found. Try our homepage.");
  const s = Bun.serve({ port: 0, fetch: () => new Response("<html><body><h1>Not found</h1></body></html>", { status: 404, headers: { "content-type": "text/html" } }) });
  servers.push(s);

  const result = await fetchContent(`http://127.0.0.1:${s.port}/missing`, opts);
  expect(result.status).toBe("error");
  expect(result.errors[0].code).toBe("FETCH_HTTP_ERROR");
});

it("still escalates a bot wall, which a browser may be served differently", async () => {
  await fakeObscuraReturning("The article body a real browser is allowed to see. ".repeat(10));
  const s = Bun.serve({ port: 0, fetch: () => new Response("<html><body>Forbidden</body></html>", { status: 403, headers: { "content-type": "text/html" } }) });
  servers.push(s);

  const result = await fetchContent(`http://127.0.0.1:${s.port}/gated`, opts);
  expect(result.status).toBe("ok");
  expect(result.method).toBe("browser");
});
