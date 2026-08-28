import { afterEach, expect, it } from "bun:test";
import { extractViaHttp } from "../../src/fetch/http-extract";
import { createDeadline } from "../../src/types";
const servers: ReturnType<typeof Bun.serve>[] = [];
afterEach(() => { while (servers.length) servers.pop()!.stop(true); });
function server(handler: (req: Request) => Response) { const s = Bun.serve({ port: 0, fetch: handler }); servers.push(s); return `http://127.0.0.1:${s.port}`; }

it("extracts readable content and metadata over http", async () => {
  const html = await Bun.file("tests/fixtures/article.html").text();
  const base = server(() => new Response(html, { headers: { "content-type": "text/html" } }));
  const result = await extractViaHttp(`${base}/article`, { deadline: createDeadline(5000), maxChars: 50_000, allowPrivateUrls: true });
  expect(result.status).toBe("ok"); expect(result.method).toBe("http"); expect(result.title).toBe("Fixture Article"); expect(result.siteName).toBe("Fixture Site"); expect(result.byline).toBe("Test Author"); expect(result.publishedTime).toBe("2026-04-26"); expect(result.excerpt).toBe("Fixture excerpt"); expect(result.text).toContain("useful paragraph");
});

it("truncates text and preserves original length", async () => {
  const html = await Bun.file("tests/fixtures/article.html").text();
  const base = server(() => new Response(html, { headers: { "content-type": "text/html" } }));
  const result = await extractViaHttp(`${base}/article`, { deadline: createDeadline(5000), maxChars: 20, allowPrivateUrls: true });
  expect(result.truncated).toBe(true); expect(result.text!.length).toBeLessThanOrEqual(20); expect(result.textLength!).toBeGreaterThan(20);
});

it("blocks redirects to private urls by default", async () => {
  const base = server(() => Response.redirect("http://127.0.0.1/private"));
  const result = await extractViaHttp(`${base}/redirect`, { deadline: createDeadline(5000), maxChars: 1000, allowPrivateUrls: true, allowPrivateRedirects: false });
  expect(result.status).toBe("error"); expect(result.errors[0].code).toBe("PRIVATE_URL_BLOCKED");
});

it("identifies as a browser so ordinary sites answer", async () => {
  // Bun's default agent string is answered with a 403 by a large share of sites,
  // which pushed every one of them onto the expensive browser tier.
  let seen = "";
  const base = server((req) => { seen = req.headers.get("user-agent") ?? ""; return new Response("<html><body><p>ok</p></body></html>", { headers: { "content-type": "text/html" } }); });
  await extractViaHttp(`${base}/a`, { deadline: createDeadline(5000), maxChars: 1000, allowPrivateUrls: true });
  expect(seen).toContain("Mozilla/5.0");
  expect(seen).not.toContain("Bun/");
});

it("reports an HTTP error status instead of parsing the error page", async () => {
  const base = server(() => new Response("<html><body>Not found</body></html>", { status: 404, headers: { "content-type": "text/html" } }));
  const result = await extractViaHttp(`${base}/missing`, { deadline: createDeadline(5000), maxChars: 1000, allowPrivateUrls: true });
  expect(result.status).toBe("error");
  expect(result.errors[0].code).toBe("FETCH_HTTP_ERROR");
  expect(result.errors[0].details).toEqual({ status: 404 });
});

it("returns plain text bodies as-is", async () => {
  const base = server(() => new Response("# Notes\n\nJust text.", { headers: { "content-type": "text/plain; charset=utf-8" } }));
  const result = await extractViaHttp(`${base}/notes.txt`, { deadline: createDeadline(5000), maxChars: 1000, allowPrivateUrls: true });
  expect(result.status).toBe("ok");
  expect(result.text).toContain("Just text.");
});

it("refuses a body larger than the cap instead of buffering it", async () => {
  const base = server(() => new Response("<html><body>" + "x".repeat(200_000) + "</body></html>", { headers: { "content-type": "text/html" } }));
  const result = await extractViaHttp(`${base}/big`, { deadline: createDeadline(5000), maxChars: 1000, allowPrivateUrls: true, maxResponseBytes: 1024 });
  expect(result.status).toBe("error");
  expect(result.errors[0].code).toBe("RESPONSE_TOO_LARGE");
});

it("rejects a non-HTML content type without downloading it", async () => {
  const base = server(() => new Response("%PDF-1.7", { headers: { "content-type": "application/pdf" } }));
  const result = await extractViaHttp(`${base}/doc.pdf`, { deadline: createDeadline(5000), maxChars: 1000, allowPrivateUrls: true });
  expect(result.status).toBe("error");
  expect(result.errors[0].code).toBe("FETCH_NOT_HTML");
  expect(result.errors[0].message).toContain("application/pdf");
});
