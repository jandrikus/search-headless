import { expect, it } from "vitest";
import { DEFAULT_CONFIG } from "../../src/config";

it("defines agent-friendly defaults", () => {
  expect(DEFAULT_CONFIG.searchLimit).toBe(10);
  expect(DEFAULT_CONFIG.fetchLimit).toBe(5);
  expect(DEFAULT_CONFIG.timeoutMs).toBe(60_000);
  expect(DEFAULT_CONFIG.maxChars).toBe(50_000);
  expect(DEFAULT_CONFIG.humanIntervention).toBe(true);
});
