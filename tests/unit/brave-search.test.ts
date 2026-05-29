import { expect, it } from "vitest";
import { detectBraveBlock, parseBraveNextUrl, parseBraveResults } from "../../src/search/parse-brave";

it("parses organic Brave results", async () => {
  const html = await Bun.file("tests/fixtures/brave-results.html").text();
  const results = parseBraveResults(html, 10);

  expect(results).toHaveLength(3); // 3 organic, 1 standalone skipped
  expect(results[0]).toMatchObject({
    rank: 1,
    title: "First Test Article - Example",
    url: "https://example.com/article-one",
    displayUrl: "example.com › article-one",
    snippet: "This is the first test article snippet with useful information about testing.",
  });
  expect(results[1]).toMatchObject({
    rank: 2,
    title: "Second Blog Post | Example Org",
    url: "https://blog.example.org/second-post",
  });
  expect(results[2]).toMatchObject({
    rank: 3,
    title: "Documentation Guide",
    url: "https://docs.example.com/guide",
  });
});

it("skips standalone snippets", async () => {
  const html = await Bun.file("tests/fixtures/brave-results.html").text();
  const results = parseBraveResults(html, 10);

  // Should not contain the standalone video result
  const urls = results.map((r) => r.url);
  expect(urls).not.toContain("https://example.com/video");
});

it("respects limit parameter", async () => {
  const html = await Bun.file("tests/fixtures/brave-results.html").text();
  const results = parseBraveResults(html, 2);

  expect(results).toHaveLength(2);
  expect(results[0].title).toBe("First Test Article - Example");
  expect(results[1].title).toBe("Second Blog Post | Example Org");
});

it("parses Brave next page URL", async () => {
  const html = await Bun.file("tests/fixtures/brave-results.html").text();
  const nextUrl = parseBraveNextUrl(html);

  expect(nextUrl).toContain("search.brave.com/search");
  expect(nextUrl).toContain("offset=1");
});

it("returns undefined when no next page", async () => {
  const html = "<html><body><div>No results</div></body></html>";
  const nextUrl = parseBraveNextUrl(html);

  expect(nextUrl).toBeUndefined();
});

it("detects Brave CAPTCHA block", async () => {
  const html = await Bun.file("tests/fixtures/brave-blocked.html").text();
  const blocked = detectBraveBlock({
    url: "https://search.brave.com/search?q=test",
    html,
    text: "Verifying you're not a bot Quick check before you continue searching.",
  });

  expect(blocked).toBe(true);
});

it("does not treat normal results as blocked", () => {
  const blocked = detectBraveBlock({
    url: "https://search.brave.com/search?q=test",
    html: "<html><body>Normal results</body></html>",
    text: "Normal results",
  });

  expect(blocked).toBe(false);
});
