use std::path::Path;

use clap::{Args, Parser, Subcommand, ValueEnum};

use search_headless::cache::CacheOptions;
use search_headless::cli::{emit, exit_code_for};
use search_headless::config;
use search_headless::fetch::{FetchContentOptions, fetch_content, fetch_content_batch};
use search_headless::output::json::render_json;
use search_headless::output::markdown::{
    render_fetch_batch_markdown, render_fetched_markdown, render_search_markdown,
};
use search_headless::search::brave::{SearchBraveOptions, search_brave};
use search_headless::types::Deadline;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Json,
    Markdown,
}

/// Web search and page content extraction for LLM coding agents.
///
/// Also answers to the names `search-web` and `fetch-content`: symlink this
/// binary under either name and it runs that subcommand directly.
#[derive(Debug, Parser)]
#[command(name = "search-headless", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search the web through Brave.
    Search(SearchArgs),
    /// Extract readable content from one or more URLs.
    Fetch(FetchArgs),
}

/// Entry point when the binary is invoked as `search-web`.
#[derive(Debug, Parser)]
#[command(name = "search-web", version, about = "Search the web through Brave.")]
struct SearchCli {
    #[command(flatten)]
    args: SearchArgs,
}

/// Entry point when the binary is invoked as `fetch-content`.
#[derive(Debug, Parser)]
#[command(name = "fetch-content", version, about = "Extract readable content from one or more URLs.")]
struct FetchCli {
    #[command(flatten)]
    args: FetchArgs,
}

#[derive(Debug, Args)]
struct SearchArgs {
    /// The search query.
    query: String,

    /// Number of results to return.
    #[arg(long, default_value_t = config::SEARCH_LIMIT, value_parser = positive_usize)]
    limit: usize,

    /// Also fetch the content of the top results.
    #[arg(long)]
    fetch: bool,

    /// How many results to fetch content for.
    #[arg(long, default_value_t = config::FETCH_LIMIT, value_parser = positive_usize)]
    fetch_limit: usize,

    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Debug, Args)]
struct FetchArgs {
    /// One or more URLs; several are fetched in parallel.
    #[arg(required = true)]
    url: Vec<String>,

    /// Truncate extracted text to this many characters.
    #[arg(long, default_value_t = config::MAX_CHARS, value_parser = positive_usize)]
    max_chars: usize,

    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Debug, Args)]
struct CommonArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Json)]
    format: Format,

    /// Overall budget for the command, in milliseconds.
    #[arg(long, default_value_t = config::TIMEOUT_MS, value_parser = positive_u64)]
    timeout: u64,

    /// Parallel page fetches.
    #[arg(long, default_value_t = config::CONCURRENCY, value_parser = positive_usize)]
    concurrency: usize,

    /// Ignore cached results and do not store new ones.
    #[arg(long = "no-cache", action = clap::ArgAction::SetFalse)]
    cache: bool,

    /// How long a cached result stays fresh, in seconds.
    #[arg(long, value_parser = positive_u64)]
    cache_ttl: Option<u64>,
}

impl CommonArgs {
    fn cache_options(&self) -> CacheOptions {
        CacheOptions {
            enabled: Some(self.cache),
            ttl_ms: self.cache_ttl.map(|seconds| seconds * 1000),
            dir: None,
        }
    }
}

fn positive_usize(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err("must be a positive integer".to_string()),
    }
}

fn positive_u64(value: &str) -> Result<u64, String> {
    match value.parse::<u64>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err("must be a positive integer".to_string()),
    }
}

/// Private URLs are blocked outright; only the test suite lifts that.
fn allow_private_urls() -> bool {
    std::env::var("SEARCH_HEADLESS_ALLOW_PRIVATE_URLS_FOR_TESTS").as_deref() == Ok("1")
}

/// Busybox-style dispatch: the name the binary was invoked under picks the
/// subcommand, so the `search-web` and `fetch-content` symlinks keep working.
fn command_from_argv0() -> Option<&'static str> {
    let argv0 = std::env::args_os().next()?;
    match Path::new(&argv0).file_stem()?.to_str()? {
        "search-web" => Some("search"),
        "fetch-content" => Some("fetch"),
        _ => None,
    }
}

fn parse_command() -> Command {
    match command_from_argv0() {
        Some("search") => Command::Search(SearchCli::parse().args),
        Some("fetch") => Command::Fetch(FetchCli::parse().args),
        _ => Cli::parse().command,
    }
}

#[tokio::main]
async fn main() {
    let code = match parse_command() {
        Command::Search(args) => run_search(args).await,
        Command::Fetch(args) => run_fetch(args).await,
    };
    std::process::exit(code);
}

async fn run_search(args: SearchArgs) -> i32 {
    let options = SearchBraveOptions {
        limit: args.limit,
        fetch: args.fetch,
        fetch_limit: args.fetch_limit,
        allow_private_urls: allow_private_urls(),
        cache: args.common.cache_options().with_default_ttl(config::SEARCH_CACHE_TTL_MS),
        concurrency: args.common.concurrency,
        ..SearchBraveOptions::new(Deadline::new(args.common.timeout))
    };

    let result = search_brave(&args.query, &options).await;
    emit(&match args.common.format {
        Format::Markdown => render_search_markdown(&result),
        Format::Json => render_json(&result),
    });
    exit_code_for(result.status)
}

async fn run_fetch(args: FetchArgs) -> i32 {
    let options = FetchContentOptions {
        max_chars: args.max_chars,
        allow_private_urls: allow_private_urls(),
        cache: args.common.cache_options().with_default_ttl(config::FETCH_CACHE_TTL_MS),
        concurrency: args.common.concurrency,
        ..FetchContentOptions::new(Deadline::new(args.common.timeout))
    };

    // One URL keeps the single-object shape callers already parse; several get a
    // batch envelope so a per-URL failure stays attached to its URL.
    if args.url.len() == 1 {
        let result = fetch_content(&args.url[0], &options).await;
        emit(&match args.common.format {
            Format::Markdown => render_fetched_markdown(&result),
            Format::Json => render_json(&result),
        });
        return exit_code_for(result.status);
    }

    let batch = fetch_content_batch(&args.url, &options).await;
    emit(&match args.common.format {
        Format::Markdown => render_fetch_batch_markdown(&batch),
        Format::Json => render_json(&batch),
    });
    exit_code_for(batch.status)
}
