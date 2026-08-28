import { JSDOM } from "jsdom";
import { Readability } from "@mozilla/readability";
import type { Deadline, FetchedContent, ToolError } from "../types";
import { remainingMs } from "../types";
import { assertPublicDns, validatePublicHttpUrl } from "./url-validation";

export interface HttpExtractOptions { deadline: Deadline; maxChars: number; allowPrivateUrls?: boolean; allowPrivateRedirects?: boolean; resolver?: (host: string) => Promise<string[]>; diagnostics?: HttpExtractDiagnostics; }

/** Filled in by extractViaHttp so callers can judge whether the result is trustworthy. */
export interface HttpExtractDiagnostics { scriptCount?: number; }

export async function extractViaHttp(input: string, options: HttpExtractOptions): Promise<FetchedContent> {
  const validation = validatePublicHttpUrl(input, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return errorResult(input, "error", validation.error);
  const dnsError = await assertPublicDns(validation.url, options);
  if (dnsError) return errorResult(input, "error", dnsError);
  try {
    const response = await fetchWithRedirects(validation.url, options);
    if (response instanceof Error) return errorResult(input, response.message === "FETCH_TIMEOUT" ? "timeout" : "error", { code: response.message, message: messageFor(response.message), recoverable: response.message === "FETCH_TIMEOUT" });
    const contentType = response.headers.get("content-type") ?? "";
    if (contentType && !contentType.includes("text/html")) return errorResult(response.url, "error", { code: "FETCH_NOT_HTML", message: `Expected HTML but got ${contentType}.`, recoverable: true });
    const html = await response.text();
    const dom = new JSDOM(html, { url: response.url });
    const doc = dom.window.document;
    // Recorded before Readability mutates the document.
    if (options.diagnostics) options.diagnostics.scriptCount = doc.querySelectorAll("script").length;
    const parsed = new Readability(doc).parse();
    const text = parsed?.textContent?.trim() || doc.body?.textContent?.replace(/\s+/g, " ").trim() || "";
    if (!text) return errorResult(response.url, "error", { code: "READABILITY_FAILED", message: "Could not extract readable text.", recoverable: true });
    const textLength = text.length;
    const truncated = textLength > options.maxChars;
    return { url: response.url, status: "ok", title: parsed?.title || doc.title || undefined, siteName: meta(doc, "og:site_name"), byline: parsed?.byline || meta(doc, "author"), publishedTime: meta(doc, "article:published_time"), excerpt: parsed?.excerpt || meta(doc, "description"), text: truncated ? text.slice(0, options.maxChars) : text, method: "http", truncated, textLength, errors: [] };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const code = msg.includes("aborted") || remainingMs(options.deadline) <= 0 ? "FETCH_TIMEOUT" : "UNKNOWN_ERROR";
    return errorResult(input, code === "FETCH_TIMEOUT" ? "timeout" : "error", { code, message: msg, recoverable: true });
  }
}

async function fetchWithRedirects(url: URL, options: HttpExtractOptions): Promise<Response | Error> {
  let current = url;
  for (let i = 0; i < 8; i++) {
    const ms = remainingMs(options.deadline); if (ms <= 0) return new Error("FETCH_TIMEOUT");
    const controller = new AbortController(); const t = setTimeout(() => controller.abort(), ms);
    const response = await fetch(current, { redirect: "manual", signal: controller.signal }); clearTimeout(t);
    if (![301,302,303,307,308].includes(response.status)) return response;
    const location = response.headers.get("location"); if (!location) return response;
    const next = new URL(location, current);
    const validation = validatePublicHttpUrl(next.toString(), { allowPrivateUrls: options.allowPrivateRedirects === true && options.allowPrivateUrls === true });
    if (!validation.ok) return new Error(validation.error.code);
    const dnsError = await assertPublicDns(validation.url, { ...options, allowPrivateUrls: options.allowPrivateRedirects === true && options.allowPrivateUrls === true });
    if (dnsError) return new Error(dnsError.code);
    current = validation.url;
  }
  return new Error("TOO_MANY_REDIRECTS");
}
function meta(doc: Document, name: string): string | undefined { return doc.querySelector(`meta[property="${name}"]`)?.getAttribute("content") || doc.querySelector(`meta[name="${name}"]`)?.getAttribute("content") || undefined; }
function messageFor(code: string) { return code === "PRIVATE_URL_BLOCKED" ? "Redirect target is private/local." : code; }
function errorResult(url: string, status: FetchedContent["status"], error: ToolError): FetchedContent { return { url, status, errors: [error] }; }
