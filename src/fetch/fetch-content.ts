import { DEFAULT_CONFIG } from "../config";
import type { Deadline, FetchedContent } from "../types";
import { createDeadline } from "../types";
import { extractViaBrowser } from "./browser-extract";
import { extractViaHttp } from "./http-extract";
import { validatePublicHttpUrl } from "./url-validation";
export interface FetchContentOptions { deadline?: Deadline; timeoutMs?: number; maxChars?: number; allowPrivateUrls?: boolean; browserFallback?: boolean; forceBrowser?: boolean; noBrowser?: boolean; headful?: boolean; }
export async function fetchContent(url: string, options: FetchContentOptions = {}): Promise<FetchedContent> {
  const deadline = options.deadline ?? createDeadline(options.timeoutMs ?? DEFAULT_CONFIG.timeoutMs);
  const maxChars = options.maxChars ?? DEFAULT_CONFIG.maxChars;
  const validation = validatePublicHttpUrl(url, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return { url, status: "error", errors: [validation.error] };
  if (options.forceBrowser) return extractViaBrowser(url, { deadline, maxChars, allowPrivateUrls: options.allowPrivateUrls, headful: options.headful });
  const http = await extractViaHttp(url, { deadline, maxChars, allowPrivateUrls: options.allowPrivateUrls, allowPrivateRedirects: false });
  if (http.status === "ok" && (http.textLength ?? 0) >= 80) return http;
  if (http.errors.some((error) => error.code === "PRIVATE_URL_BLOCKED" || error.recoverable === false)) return http;
  if (options.noBrowser || options.browserFallback === false) return http;
  const browser = await extractViaBrowser(url, { deadline, maxChars, allowPrivateUrls: options.allowPrivateUrls, headful: options.headful });
  if (browser.status === "ok") return { ...browser, status: http.status === "ok" ? "ok" : "partial", errors: http.errors };
  return browser;
}
