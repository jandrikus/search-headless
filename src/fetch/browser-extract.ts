import { chromium, type Route } from "playwright";
import { DEFAULT_CONFIG } from "../config";
import { withProfileLock } from "../browser/context";
import type { Deadline, FetchedContent } from "../types";
import { remainingMs } from "../types";
import { assertPublicDns, validatePublicHttpUrl } from "./url-validation";
export interface BrowserExtractOptions { deadline: Deadline; maxChars: number; allowPrivateUrls?: boolean; headful?: boolean; profileDir?: string; }
export function getBrowserProfileLockFile(profileDir = DEFAULT_CONFIG.profileDir): string {
  return profileDir === DEFAULT_CONFIG.profileDir ? DEFAULT_CONFIG.lockFile : `${profileDir}.lock`;
}
export async function extractViaBrowser(input: string, options: BrowserExtractOptions): Promise<FetchedContent> {
  if (remainingMs(options.deadline) <= 10) return { url: input, status: "timeout", errors: [{ code: "FETCH_TIMEOUT", message: "Overall timeout expired before browser extraction could start.", recoverable: true }] };
  const validation = validatePublicHttpUrl(input, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return { url: input, status: "error", errors: [validation.error] };
  const dnsError = await assertPublicDns(validation.url, options);
  if (dnsError) return { url: input, status: "error", errors: [dnsError] };
  let context: any;
  const profileDir = options.profileDir ?? DEFAULT_CONFIG.profileDir;
  const lockFile = getBrowserProfileLockFile(profileDir);
  try {
  return await withProfileLock(lockFile, options.deadline, async () => {
  try {
    context = await chromium.launchPersistentContext(profileDir, { headless: !options.headful });
    const page = await context.newPage();
    await page.route("**/*", async (route: Route) => {
      const req = route.request();
      if (req.isNavigationRequest()) {
        const v = validatePublicHttpUrl(req.url(), { allowPrivateUrls: options.allowPrivateUrls });
        if (!v.ok) return route.abort();
        const dns = await assertPublicDns(v.url, options);
        if (dns) return route.abort();
      }
      return route.continue();
    });
    await page.goto(validation.url.href, { waitUntil: "domcontentloaded", timeout: remainingMs(options.deadline) || 1 });
    const finalValidation = validatePublicHttpUrl(page.url(), { allowPrivateUrls: options.allowPrivateUrls });
    if (!finalValidation.ok) return { url: page.url(), status: "error", errors: [finalValidation.error] };
    const finalDns = await assertPublicDns(finalValidation.url, options);
    if (finalDns) return { url: page.url(), status: "error", errors: [finalDns] };
    await page.locator("script,style,nav,footer,aside").evaluateAll((els: Element[]) => els.forEach((el) => el.remove())).catch(() => undefined);
    const title = await page.title();
    const text = ((await page.locator("body").innerText({ timeout: 1000 }).catch(() => "")) as string).trim();
    const textLength = text.length; const truncated = textLength > options.maxChars;
    return { url: page.url(), status: text ? "ok" : "error", title, text: truncated ? text.slice(0, options.maxChars) : text, method: "browser", truncated, textLength, errors: text ? [] : [{ code: "READABILITY_FAILED", message: "Could not extract rendered text.", recoverable: true }] };
  } catch (e) { const msg = e instanceof Error ? e.message : String(e); return { url: input, status: remainingMs(options.deadline) <= 0 ? "timeout" : "error", errors: [{ code: remainingMs(options.deadline) <= 0 ? "FETCH_TIMEOUT" : "BROWSER_NAVIGATION_FAILED", message: msg, recoverable: true }] }; }
  finally { await context?.close().catch(() => undefined); }
  });
  } catch (e) {
    const error = e as { code?: string; message?: string; recoverable?: boolean };
    const timedOut = remainingMs(options.deadline) <= 0;
    return { url: input, status: timedOut ? "timeout" : "error", errors: [{ code: timedOut ? "FETCH_TIMEOUT" : error.code ?? "BROWSER_NAVIGATION_FAILED", message: error.message ?? String(e), recoverable: error.recoverable ?? true }] };
  }
}
