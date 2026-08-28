import { obscuraFetch, parseObscuraJson } from "../obscura/runner";
import type { Deadline, FetchedContent } from "../types";
import { remainingMs } from "../types";
import { assertPublicDns, validatePublicHttpUrl } from "./url-validation";

export interface ObscuraExtractOptions {
  deadline: Deadline;
  maxChars: number;
  allowPrivateUrls?: boolean;
}

/**
 * Extract title, final URL and body text.
 *
 * `noscript` is stripped alongside script/style: obscura has no layout engine, so
 * `innerText` is really `textContent` and would otherwise include `<noscript>`
 * blocks that a real browser never renders.
 */
const EXTRACT_SCRIPT = [
  `(() => {`,
  `  document.querySelectorAll('script,style,noscript,nav,footer,aside').forEach(el => el.remove());`,
  `  return JSON.stringify({title: document.title, url: window.location.href, text: document.body ? document.body.innerText : ''});`,
  `})()`,
].join("");

interface Extracted {
  title?: string;
  url?: string;
  text?: string;
}

export async function extractViaObscura(
  input: string,
  options: ObscuraExtractOptions,
): Promise<FetchedContent> {
  if (remainingMs(options.deadline) <= 1000) {
    return {
      url: input,
      status: "timeout",
      errors: [
        {
          code: "FETCH_TIMEOUT",
          message: "Overall timeout expired before obscura extraction could start.",
          recoverable: true,
        },
      ],
    };
  }

  const validation = validatePublicHttpUrl(input, { allowPrivateUrls: options.allowPrivateUrls });
  if (!validation.ok) return { url: input, status: "error", errors: [validation.error] };

  const dnsError = await assertPublicDns(validation.url, options);
  if (dnsError) return { url: input, status: "error", errors: [dnsError] };

  const result = await obscuraFetch({
    url: input,
    deadline: options.deadline,
    evalScript: EXTRACT_SCRIPT,
    stealth: true,
    allowPrivateNetwork: options.allowPrivateUrls,
  });

  if (!result.ok) {
    const timedOut = remainingMs(options.deadline) <= 0;
    return {
      url: input,
      status: timedOut ? "timeout" : "error",
      errors: [
        {
          code: timedOut ? "FETCH_TIMEOUT" : "BROWSER_NAVIGATION_FAILED",
          message: result.stderr.trim() || `obscura exited with code ${result.exitCode}.`,
          recoverable: true,
        },
      ],
    };
  }

  const parsed = parseObscuraJson<Extracted>(result.stdout);
  if (!parsed) {
    return {
      url: input,
      status: "error",
      errors: [
        {
          code: "READABILITY_FAILED",
          message: "obscura returned no usable extraction result.",
          recoverable: true,
        },
      ],
    };
  }

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
      : [{ code: "READABILITY_FAILED", message: "Could not extract rendered text.", recoverable: true }],
  };
}
