/**
 * Domain-specific URL rewrites for sites that block headless browsers.
 *
 * These rewrites are applied before any extraction attempt, allowing us to
 * bypass anti-bot protections by using alternative endpoints that serve
 * the same content without JavaScript challenges.
 */

/** Rewrite rules: match → replace. Applied in order, first match wins. */
const REWRITE_RULES: Array<{ test: (url: URL) => boolean; rewrite: (url: URL) => URL }> = [
  // Reddit: www.reddit.com serves JS anti-bot challenges to headless browsers.
  // old.reddit.com serves the same content without challenges.
  {
    test: (url) => url.hostname === "www.reddit.com",
    rewrite: (url) => {
      url.hostname = "old.reddit.com";
      return url;
    },
  },
];

/**
 * Apply domain-specific URL rewrites.
 * Returns the rewritten URL string, or the original if no rules matched.
 */
export function rewriteUrl(input: string): string {
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    return input;
  }

  for (const rule of REWRITE_RULES) {
    if (rule.test(url)) {
      return rule.rewrite(url).toString();
    }
  }

  return input;
}
