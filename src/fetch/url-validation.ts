import { lookup } from "node:dns/promises";
import type { ToolError } from "../types";
export type UrlValidation = { ok: true; url: URL } | { ok: false; error: ToolError };
export interface UrlValidationOptions { allowPrivateUrls?: boolean; }
export function validatePublicHttpUrl(input: string, options: UrlValidationOptions = {}): UrlValidation {
  let url: URL;
  try { url = new URL(input); } catch { return fail("INVALID_URL", "URL is not valid."); }
  if (url.protocol !== "http:" && url.protocol !== "https:") return fail("INVALID_URL", "Only http and https URLs are supported.");
  if (!options.allowPrivateUrls && isPrivateHost(url.hostname)) return fail("PRIVATE_URL_BLOCKED", "Local and private network URLs are blocked by default.");
  return { ok: true, url };
}
export async function assertPublicDns(url: URL, options: UrlValidationOptions & { resolver?: (host: string) => Promise<string[]> } = {}): Promise<ToolError | undefined> {
  if (options.allowPrivateUrls) return undefined;
  const resolver = options.resolver ?? (async (host: string) => (await lookup(host, { all: true })).map((a) => a.address));
  try { for (const address of await resolver(url.hostname)) if (isPrivateHost(address)) return { code: "PRIVATE_URL_BLOCKED", message: "URL resolves to a private network address.", recoverable: false }; } catch { /* DNS errors handled by fetch */ }
}
function fail(code: string, message: string): UrlValidation { return { ok: false, error: { code, message, recoverable: false } }; }
export function isPrivateHost(host: string): boolean {
  const normalized = host.toLowerCase().replace(/^\[/, "").replace(/\]$/, "");
  if (normalized === "localhost" || normalized.endsWith(".localhost")) return true;
  const mapped = normalized.match(/^::ffff:(\d+\.\d+\.\d+\.\d+)$/)?.[1];
  if (mapped) return isPrivateIpv4(mapped);
  const mappedHex = normalized.match(/^::ffff:([0-9a-f]+):([0-9a-f]+)$/i);
  if (mappedHex) {
    const high = Number.parseInt(mappedHex[1], 16);
    const low = Number.parseInt(mappedHex[2], 16);
    if (Number.isFinite(high) && Number.isFinite(low)) {
      const ipv4 = `${(high >> 8) & 255}.${high & 255}.${(low >> 8) & 255}.${low & 255}`;
      return isPrivateIpv4(ipv4);
    }
  }
  if (normalized === "::1") return true;
  if (normalized.startsWith("fc") || normalized.startsWith("fd")) return true;
  if (/^fe[89ab]/i.test(normalized)) return true;
  return isPrivateIpv4(normalized);
}
function isPrivateIpv4(host: string): boolean {
  const parts = host.split(".").map(Number);
  if (parts.length !== 4 || parts.some((p) => !Number.isInteger(p) || p < 0 || p > 255)) return false;
  const [a,b] = parts;
  return a === 0 || a === 10 || a === 127 || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168);
}
