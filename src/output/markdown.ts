import type { FetchedContent, FetchResponse, SearchResponse } from "../types";

export function renderSearchMarkdown(response: SearchResponse): string {
  const lines = [`# Search results for \`${response.query}\``, ""];
  if (response.status !== "ok") lines.push(`_Status: ${response.status}${response.cached ? " (cached)" : ""}_`, "");
  else if (response.cached) lines.push("_Served from cache._", "");

  for (const result of response.results) {
    lines.push(`${result.rank}. [${result.title}](${result.url})`);
    if (result.displayUrl) lines.push(`   - URL: ${result.displayUrl}`);
    if (result.snippet) lines.push(`   - Snippet: ${result.snippet}`);
    lines.push("");
    if (result.fetched) lines.push(...fetchedSection(result.fetched, "###"));
  }

  lines.push(...errorSection(response.errors));
  return `${lines.join("\n").trim()}\n`;
}

export function renderFetchedMarkdown(content: FetchedContent): string {
  const lines = [`# ${content.title ?? content.url}`, "", `URL: ${content.url}`, ""];
  lines.push(...provenance(content));
  if (content.excerpt) lines.push(`> ${content.excerpt}`, "");
  if (content.text) lines.push(content.text, "");
  lines.push(...errorSection(content.errors));
  return `${lines.join("\n").trim()}\n`;
}

/** One document per URL, so a failed page is visible instead of silently absent. */
export function renderFetchBatchMarkdown(response: FetchResponse): string {
  const lines = [`# Fetched ${response.results.length} URLs`, "", `_Status: ${response.status}_`, ""];
  for (const content of response.results) {
    lines.push(`## ${content.title ?? content.url}`, "", `URL: ${content.url}`, "");
    lines.push(...provenance(content));
    if (content.excerpt) lines.push(`> ${content.excerpt}`, "");
    if (content.text) lines.push(content.text, "");
    if (content.errors.length) for (const e of content.errors) lines.push(`- **${e.code}**: ${e.message}`);
    lines.push("");
  }
  return `${lines.join("\n").trim()}\n`;
}

/**
 * Fetched page bodies sit under their own heading rather than indented.
 *
 * Three-space indentation reads as a continuation to some parsers and as a code
 * block to others, and the body is the part an agent actually wants intact.
 */
function fetchedSection(content: FetchedContent, heading: string): string[] {
  const lines = [`${heading} Content of ${content.url}`, ""];
  lines.push(...provenance(content));
  if (content.text) lines.push(content.text, "");
  else if (content.errors.length) lines.push(`Could not fetch: ${content.errors[0].message}`, "");
  return lines;
}

function provenance(content: FetchedContent): string[] {
  const bits: string[] = [];
  if (content.byline) bits.push(`By ${content.byline}`);
  if (content.publishedTime) bits.push(content.publishedTime);
  if (content.method) bits.push(content.cached ? `${content.method}, cached` : content.method);
  if (content.truncated) bits.push(`truncated from ${content.textLength} chars`);
  return bits.length ? [`_${bits.join(" · ")}_`, ""] : [];
}

function errorSection(errors: { code: string; message: string }[]): string[] {
  if (!errors.length) return [];
  const lines = ["## Errors", ""];
  for (const e of errors) lines.push(`- **${e.code}**: ${e.message}`);
  return lines;
}
