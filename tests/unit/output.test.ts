import { expect, it } from "vitest";
import { renderJson } from "../../src/output/json";
import { renderFetchedMarkdown, renderSearchMarkdown } from "../../src/output/markdown";
import type { SearchResponse, FetchedContent } from "../../src/types";

const response: SearchResponse = { query: "example", engine: "google", status: "ok", results: [{ rank: 1, title: "Example", url: "https://example.com", displayUrl: "example.com", snippet: "Demo" }], errors: [] };

it("renders pretty JSON", () => expect(JSON.parse(renderJson(response))).toEqual(response));
it("renders search markdown", () => {
  const md = renderSearchMarkdown(response);
  expect(md).toContain("# Search results for `example`");
  expect(md).toContain("1. [Example](https://example.com)");
  expect(md).toContain("Demo");
});
it("renders fetched markdown", () => {
  const content: FetchedContent = { url: "https://example.com", status: "ok", title: "T", text: "Body", errors: [] };
  expect(renderFetchedMarkdown(content)).toContain("# T");
});
