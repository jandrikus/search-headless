import type { ToolError } from "../types";

export function detectGoogleBlock({ url, html, text }: { url: string; html: string; text: string }): ToolError | undefined {
  const normalizedUrl = url.toLowerCase();
  const visibleText = text.toLowerCase();
  const pageHtml = html.toLowerCase();

  const isSorryPage = normalizedUrl.includes("/sorry/");
  const isConsentPage = normalizedUrl.includes("consent.google.com") || visibleText.includes("before you continue to google");
  const isUnusualTraffic = visibleText.includes("unusual traffic") || visibleText.includes("our systems have detected");
  const hasVisibleCaptchaChallenge =
    visibleText.includes("i'm not a robot") ||
    visibleText.includes("recaptcha") ||
    visibleText.includes("captcha") ||
    pageHtml.includes("g-recaptcha") ||
    pageHtml.includes("data-sitekey");

  if (isSorryPage || isConsentPage || isUnusualTraffic || hasVisibleCaptchaChallenge) {
    return {
      code: "HUMAN_INTERVENTION_REQUIRED",
      message: "Google showed a consent, CAPTCHA, or unusual traffic page.",
      recoverable: true,
    };
  }
}
