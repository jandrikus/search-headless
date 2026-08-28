export type OutputFormat = "json" | "markdown";
export type CommandStatus = "ok" | "partial" | "blocked" | "timeout" | "error";
export type FetchMethod = "http" | "browser";

export interface ToolError { code: string; message: string; recoverable: boolean; details?: Record<string, unknown>; }
export interface Deadline { expiresAt: number; }
export function createDeadline(timeoutMs: number): Deadline { return { expiresAt: Date.now() + timeoutMs }; }
export function remainingMs(deadline: Deadline): number { return Math.max(0, deadline.expiresAt - Date.now()); }

export interface AppConfig { searchLimit: number; fetchLimit: number; timeoutMs: number; maxChars: number; humanIntervention: boolean; profileDir: string; lockFile: string; }
export interface FetchedContent { url: string; status: CommandStatus; title?: string; siteName?: string; byline?: string; publishedTime?: string; excerpt?: string; text?: string; method?: FetchMethod; truncated?: boolean; textLength?: number; errors: ToolError[]; }
export interface SearchResult { rank: number; title: string; url: string; displayUrl?: string; snippet?: string; fetched?: FetchedContent; }
export type SearchEngine = "brave";
export interface SearchResponse { query: string; engine: SearchEngine; status: CommandStatus; results: SearchResult[]; errors: ToolError[]; }
