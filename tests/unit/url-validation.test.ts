import { expect, it } from "bun:test";
import { validatePublicHttpUrl, isPrivateHost } from "../../src/fetch/url-validation";

it("accepts public http and https urls", () => {
  expect(validatePublicHttpUrl("https://example.com/a").ok).toBe(true);
  expect(validatePublicHttpUrl("http://example.com/a").ok).toBe(true);
});

it("rejects non-http urls", () => {
  const result = validatePublicHttpUrl("file:///etc/passwd");
  expect(result.ok).toBe(false);
  if (!result.ok) expect(result.error.code).toBe("INVALID_URL");
});

it("rejects local, private, metadata, and IPv6 local hosts", () => {
  for (const url of [
    "http://localhost", "http://127.0.0.1", "http://0.0.0.0", "http://10.0.0.1",
    "http://172.16.0.1", "http://192.168.1.1", "http://169.254.169.254",
    "http://[::1]", "http://[fc00::1]", "http://[fd00::1]", "http://[fe80::1]",
    "http://[::ffff:127.0.0.1]", "http://[::ffff:10.0.0.1]"
  ]) {
    const result = validatePublicHttpUrl(url);
    expect(result.ok, url).toBe(false);
    if (!result.ok) expect(result.error.code).toBe("PRIVATE_URL_BLOCKED");
  }
});

it("can allow private urls only through an internal test option", () => {
  expect(validatePublicHttpUrl("http://127.0.0.1", { allowPrivateUrls: true }).ok).toBe(true);
});
