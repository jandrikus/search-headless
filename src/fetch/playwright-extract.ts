import type { Deadline, FetchedContent } from "../types";
import { remainingMs } from "../types";
import { assertPublicDns, validatePublicHttpUrl } from "./url-validation";

export interface PlaywrightExtractOptions {
  deadline: Deadline;
  maxChars: number;
  allowPrivateUrls?: boolean;
}

/**
 * Extract page content using Playwright (full Chromium browser).
 *
 * This is the last-resort fallback when obscura and HTTP extraction both fail.
 * Playwright is a full browser that can handle JavaScript anti-bot challenges
 * that obscura's lightweight V8 environment cannot.
 *
 * Trade-offs:
 * - Pro: Can solve JS challenges, wait for navigation, handle dynamic content
 * - Con: Slower startup, higher memory usage than obscura
 */
export async function extractViaPlaywright(
  input: string,
  options: PlaywrightExtractOptions,
): Promise<FetchedContent> {
  if (remainingMs(options.deadline) <= 1000) {
    return {
      url: input,
      status: "timeout",
      errors: [
        {
          code: "FETCH_TIMEOUT",
          message: "Overall timeout expired before Playwright extraction could start.",
          recoverable: true,
        },
      ],
    };
  }

  const validation = validatePublicHttpUrl(input, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return { url: input, status: "error", errors: [validation.error] };

  const dnsError = await assertPublicDns(validation.url, options);
  if (dnsError) return { url: input, status: "error", errors: [dnsError] };

  // Dynamic import to avoid loading Playwright unless needed
  const { chromium } = await import("playwright");

  const timeout = remainingMs(options.deadline);
  let browser: import("playwright").Browser | null = null;

  try {
    browser = await chromium.launch({ headless: true });
    const context = await browser.newContext({
      userAgent:
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
      extraHTTPHeaders: { "Accept-Language": "en-US,en;q=0.9" },
    });

    const page = await context.newPage();

    // Navigate and wait for network to settle
    await page.goto(input, {
      waitUntil: "networkidle",
      timeout: Math.max(1000, timeout - 2000), // Leave 2s buffer for extraction
    });

    // Additional wait for JS challenges to complete
    await page.waitForTimeout(3000);

    // Extract content using the same pattern as obscura
    const extracted = await page.evaluate(() => {
      // Strip noisy elements
      document
        .querySelectorAll("script,style,nav,footer,aside")
        .forEach((el) => el.remove());

      return JSON.stringify({
        title: document.title,
        url: window.location.href,
        text: document.body ? document.body.innerText : "",
      });
    });

    const parsed: { title?: string; url?: string; text?: string } = JSON.parse(extracted);
    const text = (parsed.text ?? "").trim();
    const textLength = text.length;
    const truncated = textLength > options.maxChars;

    return {
      url: parsed.url || input,
      status: text ? "ok" : "error",
      title: parsed.title || undefined,
      text: truncated ? text.slice(0, options.maxChars) : text,
      method: "browser",
      truncated,
      textLength,
      errors: text
        ? []
        : [
            {
              code: "READABILITY_FAILED",
              message: "Could not extract rendered text.",
              recoverable: true,
            },
          ],
    };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const timedOut = remainingMs(options.deadline) <= 0;
    return {
      url: input,
      status: timedOut ? "timeout" : "error",
      errors: [
        {
          code: timedOut ? "FETCH_TIMEOUT" : "BROWSER_NAVIGATION_FAILED",
          message: msg,
          recoverable: true,
        },
      ],
    };
  } finally {
    await browser?.close().catch(() => undefined);
  }
}
