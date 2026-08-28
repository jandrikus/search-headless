import { DEFAULT_CONFIG } from "../config";
import type { Deadline, FetchedContent, ToolError } from "../types";
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
}

/** Below this, a script-bearing page is treated as possibly unrendered. */
const MIN_CONTENT_CHARS = 500;

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

/**
 * Extract page content with a two-tier fallback strategy:
 * 0. URL rewrite (e.g., Reddit: www → old)
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

  const validation = validatePublicHttpUrl(url, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return { url, status: "error", errors: [validation.error] };

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

  if (httpResult.status === "ok" && httpResultIsTrustworthy(httpResult, diagnostics)) return httpResult;
  if (httpResult.status !== "ok") errors.push(...httpResult.errors);

  // Tier 2: Obscura (renders JavaScript)
  if (remainingMs(deadline) > 1000) {
    const obscuraResult = await extractViaObscura(rewrittenUrl, {
      deadline,
      maxChars,
      allowPrivateUrls: options.allowPrivateUrls,
    });

    // Rendering can reveal content HTTP could not see; keep whichever saw more.
    if (obscuraResult.status === "ok") {
      if (httpResult.status === "ok" && (httpResult.textLength ?? 0) >= (obscuraResult.textLength ?? 0)) {
        return httpResult;
      }
      return obscuraResult;
    }
    errors.push(...obscuraResult.errors);
  }

  // Obscura could not improve on a usable HTTP result — return it rather than fail.
  if (httpResult.status === "ok") return httpResult;

  // All tiers failed — return combined errors
  return {
    url,
    status: timedOut(deadline, errors) ? "timeout" : "error",
    errors,
  };
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
  const concurrency = Math.max(1, options.concurrency ?? DEFAULT_CONFIG.fetchLimit);
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
