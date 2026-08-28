export type OutputFormat = "json" | "markdown";
export type CommandStatus = "ok" | "partial" | "blocked" | "timeout" | "error";
export type FetchMethod = "http" | "browser";

export interface ToolError { code: string; message: string; recoverable: boolean; details?: Record<string, unknown>; }
export interface Deadline { expiresAt: number; }
export function createDeadline(timeoutMs: number): Deadline { return { expiresAt: Date.now() + timeoutMs }; }
export function remainingMs(deadline: Deadline): number { return Math.max(0, deadline.expiresAt - Date.now()); }

export interface AppConfig {
  searchLimit: number;
  fetchLimit: number;
  timeoutMs: number;
  maxChars: number;
  /** Concurrent page fetches for multi-URL work. */
  concurrency: number;
  /** Where cached search and fetch results live. */
  cacheDir: string;
  /** How long a cached search result stays fresh, in ms. */
  searchCacheTtlMs: number;
  /** How long cached page content stays fresh, in ms. */
  fetchCacheTtlMs: number;
  /** Largest response body the HTTP tier will read, in bytes. */
  maxResponseBytes: number;
}

export interface FetchedContent { url: string; status: CommandStatus; title?: string; siteName?: string; byline?: string; publishedTime?: string; excerpt?: string; text?: string; method?: FetchMethod; truncated?: boolean; textLength?: number; cached?: boolean; errors: ToolError[]; }
export interface SearchResult { rank: number; title: string; url: string; displayUrl?: string; snippet?: string; fetched?: FetchedContent; }
export type SearchEngine = "brave";
export interface SearchResponse { query: string; engine: SearchEngine; status: CommandStatus; results: SearchResult[]; cached?: boolean; errors: ToolError[]; }

/** A `fetch-content` run over one or more URLs. */
export interface FetchResponse { status: CommandStatus; results: FetchedContent[]; errors: ToolError[]; }
