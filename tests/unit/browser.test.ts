import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { withProfileLock } from "../../src/browser/context";
import { waitForHumanIntervention } from "../../src/browser/human-intervention";
import { createDeadline } from "../../src/types";

it("prevents concurrent profile lock access", async () => {
  const dir = await mkdtemp(join(tmpdir(), "sh-lock-")); const lock = join(dir, "lock");
  await withProfileLock(lock, createDeadline(1000), async () => {
    await expect(withProfileLock(lock, createDeadline(20), async () => "nope")).rejects.toMatchObject({ code: "BROWSER_PROFILE_LOCKED" });
  });
  await rm(dir, { recursive: true, force: true });
});

it("times out waiting for human intervention", async () => {
  const result = await waitForHumanIntervention({ deadline: createDeadline(5), isResolved: async () => false, log: () => {} });
  expect(result.ok).toBe(false); if (!result.ok) expect(result.error.code).toBe("HUMAN_INTERVENTION_TIMEOUT");
});

