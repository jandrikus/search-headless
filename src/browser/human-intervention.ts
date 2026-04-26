import type { Deadline, ToolError } from "../types";
import { remainingMs } from "../types";
export async function waitForHumanIntervention(options: { deadline: Deadline; isResolved: () => Promise<boolean>; log?: (message: string) => void }): Promise<{ ok: true } | { ok: false; error: ToolError }> {
  const log = options.log ?? ((m: string) => console.error(m));
  log("Human intervention required: solve the challenge in the opened browser window. The command will continue automatically.");
  while (remainingMs(options.deadline) > 0) { if (await options.isResolved()) return { ok: true }; await Bun.sleep(Math.min(500, remainingMs(options.deadline))); }
  return { ok: false, error: { code: "HUMAN_INTERVENTION_TIMEOUT", message: "Timed out waiting for human intervention.", recoverable: true } };
}
