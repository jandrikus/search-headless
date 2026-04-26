import { homedir } from "node:os";
import { join } from "node:path";
import type { AppConfig } from "./types";
const baseDir = join(homedir(), ".search-headless");
export const DEFAULT_CONFIG: AppConfig = { searchLimit: 10, fetchLimit: 5, timeoutMs: 60_000, maxChars: 50_000, humanIntervention: true, profileDir: join(baseDir, "browser-profile"), lockFile: join(baseDir, "lock") };
