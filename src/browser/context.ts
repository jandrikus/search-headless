import { mkdir, open, rm } from "node:fs/promises";
import { dirname } from "node:path";
import { chromium } from "playwright";
import type { Deadline, ToolError } from "../types";
import { remainingMs } from "../types";
export async function withProfileLock<T>(lockFile: string, deadline: Deadline, fn: () => Promise<T>): Promise<T> {
  await mkdir(dirname(lockFile), { recursive: true }); let handle: Awaited<ReturnType<typeof open>> | undefined;
  while (remainingMs(deadline) > 0) {
    try { handle = await open(lockFile, "wx"); break; }
    catch { await Bun.sleep(Math.min(25, remainingMs(deadline))); }
  }
  if (!handle) { const err: ToolError = { code: "BROWSER_PROFILE_LOCKED", message: "Browser profile is locked by another process.", recoverable: true }; throw err; }
  try { return await fn(); } finally { await handle.close().catch(() => undefined); await rm(lockFile, { force: true }).catch(() => undefined); }
}
export function launchPersistentBrowser(options: { profileDir: string; headful?: boolean; humanIntervention?: boolean }) { return chromium.launchPersistentContext(options.profileDir, { headless: !(options.headful || options.humanIntervention) }); }
