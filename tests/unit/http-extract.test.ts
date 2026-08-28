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
