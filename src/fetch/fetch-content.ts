import { cacheRead, cacheWrite, type CacheOptions } from "../cache";
import { DEFAULT_CONFIG } from "../config";
import type { Deadline, FetchedContent, FetchResponse, ToolError } from "../types";
import { createDeadline, remainingMs } from "../types";
import type { HttpExtractDiagnostics } from "./http-extract";
import { extractViaHttp } from "./http-extract";
import { extractViaObscura } from "./obscura-extract";
import { rewriteUrl } from "./url-rewrite";
import { validatePublicHttpUrl } from "./url-validation";

export interface FetchContentOptions {
  deadline?: Deadline;
  timeoutMs?: number;
  maxChars?: number;
  allowPrivateUrls?: boolean;
  cache?: CacheOptions;
}

/** Below this, a script-bearing page is treated as possibly unrendered. */
const MIN_CONTENT_CHARS = 500;

const CACHE_NAMESPACE = "fetch";

/** HTTP-tier failures the browser tier cannot possibly fix. */
const TERMINAL_HTTP_CODES = new Set(["FETCH_NOT_HTML", "RESPONSE_TOO_LARGE", "TOO_MANY_REDIRECTS", "PRIVATE_URL_BLOCKED", "INVALID_URL"]);

/**
 * Response codes where a real browser plausibly gets a different answer.
 *
 * These are the ones that mean "not you", not "not here": a bot wall, a rate
 * limit, a login gate. A 404 or a 500 is the same page in any client, and
 * rendering it just returns the error page's own text as if it were the article.
 */
const ESCALATE_STATUSES = new Set([401, 403, 405, 406, 429, 451]);

/** Should the browser tier see this HTTP-tier failure at all? */
function worthRendering(errors: ToolError[]): boolean {
  for (const error of errors) {
    if (TERMINAL_HTTP_CODES.has(error.code)) return false;
    if (error.code === "FETCH_HTTP_ERROR") {
      const status = Number(error.details?.status);
      if (!ESCALATE_STATUSES.has(status)) return false;
    }
  }
  return true;
}

/**
 * Decide whether an exhausted set of tiers should read as a timeout.
 *
 * Checking the clock alone is racy: with a very short deadline every tier can
 * decline to start and still leave a millisecond on the clock.
 */
function timedOut(deadline: Deadline, errors: ToolError[]): boolean {
  if (remainingMs(deadline) <= 0) return true;
  return errors.length > 0 && errors.every((error) => error.code === "FETCH_TIMEOUT");
}

/**
 * Is the HTTP result good enough to skip the browser?
 *
 * A JavaScript shell still yields text — nav links, a "Loading…" placeholder, a
 * cookie banner — and Readability happily returns it, so length alone would let
 * an unrendered page through as success. Escalate only when the page carries
 * script that could have produced more: a script-free page has nothing further
 * to render, however short it is.
 */
function httpResultIsTrustworthy(result: FetchedContent, diagnostics: HttpExtractDiagnostics): boolean {
  if ((result.textLength ?? 0) >= MIN_CONTENT_CHARS) return true;
  return (diagnostics.scriptCount ?? 0) === 0;
}

/** Truncation happens per request, so the cap belongs in the key. */
function cacheKey(url: string, maxChars: number): string {
  return `${maxChars}:${url}`;
}

/**
 * Extract page content with a two-tier fallback strategy:
 * 0. Cache, then URL rewrite (e.g., Reddit: www → old)
 * 1. HTTP + Readability (plain HTTP, no browser)
 * 2. Obscura (headless browser, for JS-rendered pages)
 *
 * HTTP goes first because it is both cheaper and higher quality: Readability
 * yields separated paragraphs plus byline/excerpt/publishedTime metadata, while
 * obscura has no layout engine and so returns `textContent` with no metadata.
 * Obscura handles what HTTP cannot — pages whose content is rendered by script —
 * and a thin HTTP result on a scripted page is re-checked against it rather than
 * returned as success.
 */
export async function fetchContent(
  url: string,
  options: FetchContentOptions = {},
): Promise<FetchedContent> {
  const deadline = options.deadline ?? createDeadline(options.timeoutMs ?? DEFAULT_CONFIG.timeoutMs);
  const maxChars = options.maxChars ?? DEFAULT_CONFIG.maxChars;
  const cacheOptions: CacheOptions = { ttlMs: DEFAULT_CONFIG.fetchCacheTtlMs, ...options.cache };

  const validation = validatePublicHttpUrl(url, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return { url, status: "error", errors: [validation.error] };

  const cached = cacheRead<FetchedContent>(CACHE_NAMESPACE, cacheKey(url, maxChars), cacheOptions);
  if (cached) return { ...cached, cached: true };

  // Tier 0: Apply domain-specific URL rewrites (e.g., Reddit www → old)
  const rewrittenUrl = rewriteUrl(url);
  const errors: ToolError[] = [];

  // Tier 1: HTTP + Readability (cheap, best text and metadata)
  const diagnostics: HttpExtractDiagnostics = {};
  const httpResult = await extractViaHttp(rewrittenUrl, {
    deadline,
    maxChars,
    allowPrivateUrls: options.allowPrivateUrls,
    diagnostics,
  });

  if (httpResult.status === "ok" && httpResultIsTrustworthy(httpResult, diagnostics)) {
    return store(url, maxChars, httpResult, cacheOptions);
  }
  if (httpResult.status !== "ok") errors.push(...httpResult.errors);

  // A PDF stays a PDF and a 404 stays a 404; only failures a browser might see
  // differently are worth the cost of rendering.
  const terminal = !worthRendering(httpResult.errors);

  // Tier 2: Obscura (renders JavaScript)
  if (!terminal && remainingMs(deadline) > 1000) {
    const obscuraResult = await extractViaObscura(rewrittenUrl, {
      deadline,
      maxChars,
      allowPrivateUrls: options.allowPrivateUrls,
    });

    // Rendering can reveal content HTTP could not see; keep whichever saw more.
    if (obscuraResult.status === "ok") {
      if (httpResult.status === "ok" && (httpResult.textLength ?? 0) >= (obscuraResult.textLength ?? 0)) {
        return store(url, maxChars, httpResult, cacheOptions);
      }
      return store(url, maxChars, obscuraResult, cacheOptions);
    }
    errors.push(...obscuraResult.errors);
  }

  // Obscura could not improve on a usable HTTP result — return it rather than fail.
  if (httpResult.status === "ok") return store(url, maxChars, httpResult, cacheOptions);

  // All tiers failed — return combined errors
  return {
    url,
    status: timedOut(deadline, errors) ? "timeout" : "error",
    errors,
  };
}

/** Cache a successful result under the URL the caller asked for, and return it. */
function store(url: string, maxChars: number, result: FetchedContent, cacheOptions: CacheOptions): FetchedContent {
  cacheWrite(CACHE_NAMESPACE, cacheKey(url, maxChars), result, cacheOptions);
  return result;
}

/**
 * Fetch several URLs concurrently, preserving input order.
 *
 * Both tiers parallelize well: HTTP is I/O-bound, and each obscura fetch is its
 * own process.
 */
export async function fetchManyContent(
  urls: string[],
  options: FetchContentOptions & { concurrency?: number } = {},
): Promise<FetchedContent[]> {
  const concurrency = Math.max(1, options.concurrency ?? DEFAULT_CONFIG.concurrency);
  const results = new Array<FetchedContent>(urls.length);
  let next = 0;

  const worker = async () => {
    while (true) {
      const index = next++;
      if (index >= urls.length) return;
      results[index] = await fetchContent(urls[index], options);
    }
  };

  await Promise.all(Array.from({ length: Math.min(concurrency, urls.length) }, worker));
  return results;
}

/**
 * Fetch a batch and roll the outcomes into one response.
 *
 * `partial` means at least one URL worked: an agent asking for five pages and
 * getting four should still get the four, and a non-zero exit only when nothing
 * came back at all.
 */
export async function fetchContentBatch(
  urls: string[],
  options: FetchContentOptions & { concurrency?: number } = {},
): Promise<FetchResponse> {
  const deadline = options.deadline ?? createDeadline(options.timeoutMs ?? DEFAULT_CONFIG.timeoutMs);
  const results = await fetchManyContent(urls, { ...options, deadline });
  const okCount = results.filter((result) => result.status === "ok").length;

  let status: FetchResponse["status"];
  if (okCount === results.length) status = "ok";
  else if (okCount > 0) status = "partial";
  else if (results.every((result) => result.status === "timeout")) status = "timeout";
  else status = "error";

  return {
    status,
    results,
    errors: okCount === results.length ? [] : results.flatMap((result) => result.errors),
  };
}
