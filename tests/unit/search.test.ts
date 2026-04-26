import { expect, it } from "vitest";
import { detectGoogleBlock } from "../../src/search/block-detection";
import { parseGoogleNextUrl, parseGoogleResults } from "../../src/search/parse-results";

it("detects google block and consent pages", async () => {
  expect(detectGoogleBlock({ url: "https://www.google.com/sorry/index", html: "", text: "" })?.code).toBe("HUMAN_INTERVENTION_REQUIRED");
  expect(detectGoogleBlock({ url: "https://consent.google.com/", html: await Bun.file("tests/fixtures/google-consent.html").text(), text: "Before you continue to Google" })?.code).toBe("HUMAN_INTERVENTION_REQUIRED");
  expect(detectGoogleBlock({ url: "https://google.com", html: await Bun.file("tests/fixtures/google-blocked.html").text(), text: "unusual traffic" })?.code).toBe("HUMAN_INTERVENTION_REQUIRED");
});

it("does not treat normal results with captcha-related scripts as blocked", async () => {
  const html = `<html><head><script src="/recaptcha/enterprise.js"></script></head><body><div id="search"><a href="https://example.com"><h3>Normal Result</h3></a></div></body></html>`;
  expect(detectGoogleBlock({ url: "https://www.google.com/search?q=test", html, text: "Normal Result" })).toBeUndefined();
});

it("parses organic google results", async () => {
  const results = parseGoogleResults(await Bun.file("tests/fixtures/google-results.html").text(), 10);
  expect(results).toHaveLength(2); expect(results[0]).toMatchObject({ rank: 1, title: "Alpha", url: "https://example.com/a", snippet: "Alpha snippet" }); expect(results[1].rank).toBe(2);
  expect(parseGoogleNextUrl(await Bun.file("tests/fixtures/google-results.html").text())).toContain("start=10");
});
