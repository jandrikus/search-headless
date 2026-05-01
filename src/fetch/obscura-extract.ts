import type { Deadline, FetchedContent } from "../types";
import { remainingMs } from "../types";
import { assertPublicDns, validatePublicHttpUrl } from "./url-validation";

export interface ObscuraExtractOptions {
  deadline: Deadline;
  maxChars: number;
  allowPrivateUrls?: boolean;
}

export async function extractViaObscura(
  input: string,
  options: ObscuraExtractOptions,
): Promise<FetchedContent> {
  if (remainingMs(options.deadline) <= 10) {
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

  const timeout = remainingMs(options.deadline);
  const waitSecs = Math.max(1, Math.ceil(timeout / 1000));

  // Extract title, final URL (after redirects), and body text.
  // Strip noisy elements before extraction, matching the old Playwright behavior.
  const evalScript = [
    `(() => {`,
    `  document.querySelectorAll('script,style,nav,footer,aside').forEach(el => el.remove());`,
    `  return JSON.stringify({title: document.title, url: window.location.href, text: document.body ? document.body.innerText : ''});`,
    `})()`,
  ].join("");

  try {
    const proc = Bun.spawn(
      [
        "obscura",
        "fetch",
        input,
        "--quiet",
        "--stealth",
        "--wait",
        String(waitSecs),
        "--eval",
        evalScript,
      ],
      {
        stdout: "pipe",
        stderr: "pipe",
        timeout: Math.max(1000, timeout),
      },
    );

    const stdout = await new Response(proc.stdout).text();
    const exitCode = await proc.exited;

    if (exitCode !== 0 || !stdout.trim()) {
      const status = remainingMs(options.deadline) <= 0 ? "timeout" : "error";
      return {
        url: input,
        status,
        errors: [
          {
            code: status === "timeout" ? "FETCH_TIMEOUT" : "BROWSER_NAVIGATION_FAILED",
            message: `obscura exited with code ${exitCode}.`,
            recoverable: true,
          },
        ],
      };
    }

    let parsed: { title?: string; url?: string; text?: string };
    try {
      parsed = JSON.parse(stdout.trim());
    } catch {
      // eval didn't produce valid JSON — use raw output as text
      parsed = { text: stdout.trim() };
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
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const timedOut = remainingMs(options.deadline) <= 0;
    return {
      url: input,
      status: timedOut ? "timeout" : "error",
      errors: [{ code: timedOut ? "FETCH_TIMEOUT" : "BROWSER_NAVIGATION_FAILED", message: msg, recoverable: true }],
    };
  }
}
