import { JSDOM } from "jsdom";
import { Readability } from "@mozilla/readability";
import { DEFAULT_CONFIG } from "../config";
import type { Deadline, FetchedContent, ToolError } from "../types";
import { remainingMs } from "../types";
import { assertPublicDns, validatePublicHttpUrl } from "./url-validation";

export interface HttpExtractOptions { deadline: Deadline; maxChars: number; allowPrivateUrls?: boolean; allowPrivateRedirects?: boolean; resolver?: (host: string) => Promise<string[]>; diagnostics?: HttpExtractDiagnostics; maxResponseBytes?: number; }

/** Filled in by extractViaHttp so callers can judge whether the result is trustworthy. */
export interface HttpExtractDiagnostics { scriptCount?: number; contentType?: string; bytes?: number; }

/**
 * Headers a plain `fetch` does not send.
 *
 * Bun identifies itself as `Bun/x.y` by default, and a large share of sites
 * answer an unknown agent with a 403 or a consent wall. Sending what an ordinary
 * browser sends is not evasion — it asks for the same page a person would get,
 * and it is the difference between the cheap tier working and every fetch paying
 * for a browser launch.
 */
const REQUEST_HEADERS: Record<string, string> = {
  "user-agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
  accept: "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
  "accept-language": "en-US,en;q=0.9",
  "cache-control": "no-cache",
};

const HTML_TYPES = ["text/html", "application/xhtml+xml"];

export async function extractViaHttp(input: string, options: HttpExtractOptions): Promise<FetchedContent> {
  const validation = validatePublicHttpUrl(input, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return errorResult(input, "error", validation.error);
  const dnsError = await assertPublicDns(validation.url, options);
  if (dnsError) return errorResult(input, "error", dnsError);
  try {
    const response = await fetchWithRedirects(validation.url, options);
    if (response instanceof Error) return errorResult(input, response.message === "FETCH_TIMEOUT" ? "timeout" : "error", { code: response.message, message: messageFor(response.message), recoverable: response.message === "FETCH_TIMEOUT" });
    if (response.status >= 400) return errorResult(response.url, "error", { code: "FETCH_HTTP_ERROR", message: `Server responded ${response.status} ${response.statusText}.`.trim(), recoverable: response.status >= 500 || response.status === 429, details: { status: response.status } });

    const contentType = (response.headers.get("content-type") ?? "").toLowerCase();
    if (options.diagnostics) options.diagnostics.contentType = contentType;
    const isHtml = contentType === "" || HTML_TYPES.some((type) => contentType.includes(type));
    const isPlainText = contentType.includes("text/plain") || contentType.includes("text/markdown");
    if (!isHtml && !isPlainText) return errorResult(response.url, "error", { code: "FETCH_NOT_HTML", message: `Expected HTML but got ${contentType || "an unknown content type"}.`, recoverable: false, details: { contentType } });

    const body = await readBounded(response, options.maxResponseBytes ?? DEFAULT_CONFIG.maxResponseBytes);
    if (body instanceof Error) return errorResult(response.url, "error", { code: body.message, message: body.message === "RESPONSE_TOO_LARGE" ? "Response exceeded the size limit." : body.message, recoverable: false });
    if (options.diagnostics) options.diagnostics.bytes = body.length;

    // Plain text needs no article extraction; it is already the article.
    if (isPlainText) {
      const text = body.trim();
      if (!text) return errorResult(response.url, "error", { code: "READABILITY_FAILED", message: "Response body was empty.", recoverable: true });
      if (options.diagnostics) options.diagnostics.scriptCount = 0;
      return withText({ url: response.url, title: undefined }, text, options);
    }

    const dom = new JSDOM(body, { url: response.url });
    const doc = dom.window.document;
    // Recorded before Readability mutates the document.
    if (options.diagnostics) options.diagnostics.scriptCount = doc.querySelectorAll("script").length;
    const title = doc.title || undefined;
    const siteName = meta(doc, "og:site_name");
    const authorMeta = meta(doc, "author");
    const publishedTime = meta(doc, "article:published_time");
    const description = meta(doc, "description");
    const parsed = new Readability(doc).parse();
    const text = parsed?.textContent?.trim() || doc.body?.textContent?.replace(/\s+/g, " ").trim() || "";
    if (!text) return errorResult(response.url, "error", { code: "READABILITY_FAILED", message: "Could not extract readable text.", recoverable: true });
    return withText({ url: response.url, title: parsed?.title || title, siteName, byline: parsed?.byline || authorMeta, publishedTime, excerpt: parsed?.excerpt || description }, text, options);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const code = msg.includes("aborted") || remainingMs(options.deadline) <= 0 ? "FETCH_TIMEOUT" : "UNKNOWN_ERROR";
    return errorResult(input, code === "FETCH_TIMEOUT" ? "timeout" : "error", { code, message: msg, recoverable: true });
  }
}

/**
 * Read the body, stopping once it exceeds the cap.
 *
 * `response.text()` on a multi-gigabyte file buffers the whole thing before
 * anyone can object, so the cap has to be enforced while reading.
 */
async function readBounded(response: Response, maxBytes: number): Promise<string | Error> {
  const declared = Number(response.headers.get("content-length"));
  if (Number.isFinite(declared) && declared > maxBytes) return new Error("RESPONSE_TOO_LARGE");
  if (!response.body) return "";

  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel();
        return new Error("RESPONSE_TOO_LARGE");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }

  const joined = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) { joined.set(chunk, offset); offset += chunk.byteLength; }
  return new TextDecoder("utf-8", { fatal: false }).decode(joined);
}

function withText(base: Omit<FetchedContent, "status" | "errors">, text: string, options: HttpExtractOptions): FetchedContent {
  const textLength = text.length;
  const truncated = textLength > options.maxChars;
  return { ...base, status: "ok", text: truncated ? text.slice(0, options.maxChars) : text, method: "http", truncated, textLength, errors: [] };
}

async function fetchWithRedirects(url: URL, options: HttpExtractOptions): Promise<Response | Error> {
  let current = url;
  for (let i = 0; i < 8; i++) {
    const ms = remainingMs(options.deadline); if (ms <= 0) return new Error("FETCH_TIMEOUT");
    const controller = new AbortController(); const t = setTimeout(() => controller.abort(), ms);
    const response = await fetch(current, { redirect: "manual", signal: controller.signal, headers: REQUEST_HEADERS }); clearTimeout(t);
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
