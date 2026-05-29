import { DEFAULT_CONFIG } from "../config";
import type { Deadline, FetchedContent, ToolError } from "../types";
import { createDeadline, remainingMs } from "../types";
import { extractViaHttp } from "./http-extract";
import { extractViaObscura } from "./obscura-extract";
import { extractViaPlaywright } from "./playwright-extract";
import { rewriteUrl } from "./url-rewrite";
import { validatePublicHttpUrl } from "./url-validation";

export interface FetchContentOptions {
  deadline?: Deadline;
  timeoutMs?: number;
  maxChars?: number;
  allowPrivateUrls?: boolean;
}

/**
 * Extract page content with a three-tier fallback strategy:
 * 1. URL rewrite (e.g., Reddit: www → old)
 * 2. Obscura (fast, lightweight headless browser)
 * 3. HTTP + Readability (plain HTTP, no JS rendering)
 * 4. Playwright (full browser, last resort)
 *
 * Each tier falls back to the next when it returns empty text or an error.
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

  // Tier 1: Try obscura (fast, lightweight)
  const obscuraResult = await extractViaObscura(rewrittenUrl, {
    deadline,
    maxChars,
    allowPrivateUrls: options.allowPrivateUrls,
  });
  if (obscuraResult.status === "ok") return obscuraResult;

  // Collect errors from failed attempts
  const errors: ToolError[] = [...obscuraResult.errors];

  // Tier 2: Try HTTP + Readability (plain HTTP, no JS rendering)
  if (remainingMs(deadline) > 1000) {
    const httpResult = await extractViaHttp(rewrittenUrl, {
      deadline,
      maxChars,
      allowPrivateUrls: options.allowPrivateUrls,
    });
    if (httpResult.status === "ok") return httpResult;
    errors.push(...httpResult.errors);
  }

  // Tier 3: Try Playwright (full browser, last resort)
  if (remainingMs(deadline) > 1000) {
    const playwrightResult = await extractViaPlaywright(rewrittenUrl, {
      deadline,
      maxChars,
      allowPrivateUrls: options.allowPrivateUrls,
    });
    if (playwrightResult.status === "ok") return playwrightResult;
    errors.push(...playwrightResult.errors);
  }

  // All tiers failed — return combined errors
  const timedOut = remainingMs(deadline) <= 0;
  return {
    url,
    status: timedOut ? "timeout" : "error",
    errors,
  };
}
