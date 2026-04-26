import type { FetchedContent, SearchResponse } from "../types";
export function renderSearchMarkdown(response: SearchResponse): string {
  const lines = [`# Search results for \`${response.query}\``, ""];
  for (const result of response.results) {
    lines.push(`${result.rank}. [${result.title}](${result.url})`);
    if (result.displayUrl) lines.push(`   - URL: ${result.displayUrl}`);
    if (result.snippet) lines.push(`   - Snippet: ${result.snippet}`);
    if (result.fetched?.text) lines.push("", "   ## Fetched content", "", indent(result.fetched.text, "   "));
    lines.push("");
  }
  if (response.errors.length) { lines.push("## Errors", ""); for (const e of response.errors) lines.push(`- ${e.code}: ${e.message}`); }
  return `${lines.join("\n").trim()}\n`;
}
export function renderFetchedMarkdown(content: FetchedContent): string {
  const lines = [`# ${content.title ?? content.url}`, "", `URL: ${content.url}`, ""];
  if (content.excerpt) lines.push(`> ${content.excerpt}`, "");
  if (content.text) lines.push(content.text, "");
  if (content.errors.length) { lines.push("## Errors", ""); for (const e of content.errors) lines.push(`- ${e.code}: ${e.message}`); }
  return `${lines.join("\n").trim()}\n`;
}
function indent(text: string, prefix: string) { return text.split("\n").map((l) => `${prefix}${l}`).join("\n"); }
