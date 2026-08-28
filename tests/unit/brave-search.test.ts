import { expect, it } from "bun:test";
import { detectBraveBlock, parseBraveNextUrl, parseBraveResults } from "../../src/search/parse-brave";
import { canonicalUrl } from "../../src/search/brave";

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
    html,
    text: "Verifying you're not a bot Quick check before you continue searching.",
  });

  expect(blocked).toBe(true);
});

it("does not treat normal results as blocked", () => {
  const blocked = detectBraveBlock({
    html: "<html><body>Normal results</body></html>",
    text: "Normal results",
  });

  expect(blocked).toBe(false);
});

it("detects Brave interstitials that are not the CAPTCHA wording", async () => {
  for (const text of ["Your request has been flagged.", "This page needs JavaScript to run.", "unusual traffic from your network"]) {
    expect(detectBraveBlock({ html: "<html><body></body></html>", text })).toBe(true);
  }
});

it("does not report a block when results parsed, even if a snippet mentions a phrase", async () => {
  const html = await Bun.file("tests/fixtures/brave-results.html").text();
  expect(detectBraveBlock({ html, text: "verifying you can use this library" })).toBe(false);
});

it("canonicalises urls so the same result is not ranked twice", () => {
  const a = canonicalUrl("https://www.Example.com/post/?utm_source=brave&id=7#top");
  const b = canonicalUrl("https://example.com/post?id=7");
  expect(a).toBe(b);
});

it("keeps genuinely different urls apart", () => {
  expect(canonicalUrl("https://example.com/a")).not.toBe(canonicalUrl("https://example.com/b"));
  expect(canonicalUrl("https://example.com/a?page=1")).not.toBe(canonicalUrl("https://example.com/a?page=2"));
});

it("passes an unparseable url through unchanged", () => {
  expect(canonicalUrl("not a url")).toBe("not a url");
});
