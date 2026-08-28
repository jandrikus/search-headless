import type { Deadline } from "../types";
import { remainingMs } from "../types";

/**
 * Single place where `obscura` is invoked.
 *
 * Every call site goes through here so argument construction, deadline clamping
 * and failure reporting stay consistent, and so tests can point `OBSCURA_BIN` at
 * a fake binary and assert the arguments without needing obscura installed.
 */

/**
 * Upper bound for `--wait`, in seconds.
 *
 * `--wait` is how long obscura settles the page after load, so scaling it to the
 * whole remaining deadline (as the original code did) makes a generous timeout
 * translate into a minute of idling per page.
 */
const MAX_WAIT_SECS = 8;

export interface ObscuraFetchArgs {
  url: string;
  deadline: Deadline;
  /** JS evaluated in the page; its return value becomes stdout. */
  evalScript?: string;
  stealth?: boolean;
  waitSecs?: number;
  allowPrivateNetwork?: boolean;
}

export interface ObscuraFetchResult {
  ok: boolean;
  stdout: string;
  stderr: string;
  exitCode: number;
}

/** Resolve the obscura binary, honouring the test override. */
export function obscuraBin(): string {
  return process.env.OBSCURA_BIN ?? "obscura";
}

/** Seconds obscura should settle the page, clamped to the remaining deadline. */
export function resolveWaitSecs(deadline: Deadline, requested?: number): number {
  const budget = Math.floor(remainingMs(deadline) / 1000);
  const wanted = requested ?? MAX_WAIT_SECS;
  return Math.max(1, Math.min(MAX_WAIT_SECS, wanted, budget || 1));
}

export function buildObscuraArgs(args: ObscuraFetchArgs): string[] {
  const timeoutSecs = Math.max(1, Math.ceil(remainingMs(args.deadline) / 1000));
  const cmd = [obscuraBin(), "fetch", args.url, "--quiet"];
  if (args.stealth) cmd.push("--stealth");
  cmd.push("--wait", String(resolveWaitSecs(args.deadline, args.waitSecs)));
  cmd.push("--timeout", String(timeoutSecs));
  if (args.allowPrivateNetwork) cmd.push("--allow-private-network");
  if (args.evalScript) cmd.push("--eval", args.evalScript);
  return cmd;
}

/**
 * Run `obscura fetch` once.
 *
 * Never throws on a non-zero exit; the caller inspects the result. A missing
 * binary surfaces as `exitCode: -1` with the spawn message on stderr.
 */
export async function obscuraFetch(args: ObscuraFetchArgs): Promise<ObscuraFetchResult> {
  const timeoutMs = Math.max(1000, remainingMs(args.deadline));

  let proc: ReturnType<typeof Bun.spawn>;
  try {
    proc = Bun.spawn(buildObscuraArgs(args), { stdout: "pipe", stderr: "pipe", timeout: timeoutMs });
  } catch (e) {
    return { ok: false, stdout: "", stderr: e instanceof Error ? e.message : String(e), exitCode: -1 };
  }

  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(proc.stdout as ReadableStream).text(),
    new Response(proc.stderr as ReadableStream).text(),
    proc.exited,
  ]);

  return { ok: exitCode === 0 && stdout.trim().length > 0, stdout, stderr, exitCode };
}

/**
 * Parse obscura's stdout as JSON.
 *
 * `--eval` prints whatever the script returned, so a script that throws yields
 * empty or non-JSON output rather than an error exit. Returns null in that case.
 */
export function parseObscuraJson<T>(stdout: string): T | null {
  const trimmed = stdout.trim();
  if (!trimmed) return null;
  try {
    const parsed = JSON.parse(trimmed);
    return parsed && typeof parsed === "object" ? (parsed as T) : null;
  } catch {
    return null;
  }
}
