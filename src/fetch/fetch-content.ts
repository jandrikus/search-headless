import { DEFAULT_CONFIG } from "../config";
import type { Deadline, FetchedContent } from "../types";
import { createDeadline } from "../types";
import { extractViaObscura } from "./obscura-extract";
import { validatePublicHttpUrl } from "./url-validation";
export interface FetchContentOptions { deadline?: Deadline; timeoutMs?: number; maxChars?: number; allowPrivateUrls?: boolean; }
export async function fetchContent(url: string, options: FetchContentOptions = {}): Promise<FetchedContent> {
  const deadline = options.deadline ?? createDeadline(options.timeoutMs ?? DEFAULT_CONFIG.timeoutMs);
  const maxChars = options.maxChars ?? DEFAULT_CONFIG.maxChars;
  const validation = validatePublicHttpUrl(url, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return { url, status: "error", errors: [validation.error] };
  return extractViaObscura(url, { deadline, maxChars, allowPrivateUrls: options.allowPrivateUrls });
}
