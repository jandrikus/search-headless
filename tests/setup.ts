import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/**
 * Keep the suite off the developer's real cache and rate-limit files.
 *
 * These are process-wide env vars, so the CLI subprocesses the integration tests
 * spawn inherit them too. Without this a cached result from a previous run would
 * silently answer a test, and running the suite would space itself out ten
 * seconds per search.
 */
const sandbox = mkdtempSync(join(tmpdir(), "search-headless-test-"));

process.env.SEARCH_HEADLESS_CACHE_DIR = join(sandbox, "cache");
process.env.SEARCH_HEADLESS_RATE_FILE = join(sandbox, "last-search");
process.env.SEARCH_HEADLESS_NO_RATE_LIMIT = "1";
