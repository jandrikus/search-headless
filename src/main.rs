use clap::{Args, Parser, Subcommand, ValueEnum};

use search_headless::cache::CacheOptions;
use search_headless::cli::{emit, exit_code_for};
use search_headless::config;
use search_headless::fetch::{FetchContentOptions, fetch_content, fetch_content_batch};
use search_headless::output::json::render_json;
use search_headless::output::markdown::{render_fetch_batch_markdown, render_fetched_markdown, render_search_markdown};
use search_headless::search::brave::{SearchBraveOptions, search_brave};
use search_headless::search::searxng::{SearchSearxngOptions, search_searxng};
use search_headless::types::Deadline;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Json,
    Markdown,
}

/// Web search and page content extraction for LLM coding agents.
#[derive(Debug, Parser)]
#[command(name = "search-headless", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search through Brave or a configured SearXNG instance.
    Search(SearchArgs),
    /// Extract readable content from one or more URLs.
    Fetch(FetchArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SearchBackend {
    Brave,
    Searxng,
}

#[derive(Debug, Args)]
struct SearchArgs {
    /// The search query.
    query: String,

    /// Search backend. Pi configures this explicitly; the CLI keeps its Brave default.
    #[arg(long, value_enum, default_value_t = SearchBackend::Brave)]
    engine: SearchBackend,

    /// SearXNG base URL. Defaults to SEARXNG_URL; credentials come only from SEARXNG_USER and SEARXNG_PASS.
    #[arg(long)]
    searxng_url: Option<String>,

    /// Maximum results to return. Reads only the first page unless --paginate is set.
    #[arg(long, default_value_t = config::SEARCH_LIMIT, value_parser = positive_usize)]
    limit: usize,

    /// Follow additional result pages until the limit is reached.
    #[arg(long)]
    paginate: bool,

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
        CacheOptions { enabled: Some(self.cache), ttl_ms: self.cache_ttl.map(|seconds| seconds * 1000), dir: None }
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

#[tokio::main]
async fn main() {
    let code = match Cli::parse().command {
        Command::Search(args) => run_search(args).await,
        Command::Fetch(args) => run_fetch(args).await,
    };
    std::process::exit(code);
}

async fn run_search(args: SearchArgs) -> i32 {
    let options = SearchBraveOptions {
        limit: args.limit,
        paginate: args.paginate,
        fetch: args.fetch,
        fetch_limit: args.fetch_limit,
        allow_private_urls: allow_private_urls(),
        cache: args.common.cache_options().with_default_ttl(config::SEARCH_CACHE_TTL_MS),
        concurrency: args.common.concurrency,
        ..SearchBraveOptions::new(Deadline::new(args.common.timeout))
    };

    let result = match args.engine {
        SearchBackend::Brave => search_brave(&args.query, &options).await,
        SearchBackend::Searxng => {
            search_searxng(
                &args.query,
                &SearchSearxngOptions {
                    instance_url: args.searxng_url,
                    limit: options.limit,
                    paginate: options.paginate,
                    fetch: options.fetch,
                    fetch_limit: options.fetch_limit,
                    deadline: options.deadline,
                    cache: options.cache,
                    concurrency: options.concurrency,
                    allow_private_urls: options.allow_private_urls,
                },
            )
            .await
        }
    };
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
