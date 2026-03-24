mod config;
mod notion;
mod output;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process;

use anyhow::{Context, Result, anyhow, bail};
use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum, error::ErrorKind};
use rpassword::prompt_password;
use serde_json::{Value, json};
use tracing_subscriber::EnvFilter;

use crate::config::{
    AuthType, ConfigStore, ProfileMeta, StoredSecret, read_text_file, slugify_profile_name,
};
use crate::notion::{
    CreateParent, NotionClient, SearchFilter, bot_id_from_user_me, normalize_notion_id,
    object_kind, owner_email_from_user_me, owner_name_from_user_me,
};
use crate::output::OutputFormat;

#[derive(Parser)]
#[command(
    name = "notioncli",
    version,
    about = "A structured Rust CLI for the Notion API",
    long_about = "A structured Rust CLI for the Notion API.\n\nRun `notioncli auth login` once to store your integration token in the local credentials file. Future commands reuse the active stored profile automatically. Set `NOTION_TOKEN` for one-shot use without saving."
)]
struct Cli {
    /// Use a specific saved profile instead of the active profile.
    #[arg(long, global = true)]
    profile: Option<String>,
    /// Output format for command results. The default is human-readable.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    output: OutputFormat,
    /// Increase log verbosity. Repeat for more detail.
    #[arg(long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone)]
struct GlobalOptions {
    profile: Option<String>,
    output: OutputFormat,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Manage saved credentials and inspect authentication state")]
    Auth(AuthArgs),
    #[command(about = "Search pages and data sources in the current workspace")]
    Search(SearchArgs),
    #[command(about = "Read and mutate page content, metadata, and markdown")]
    Page(PageArgs),
    #[command(name = "data-source")]
    #[command(about = "Inspect, query, create, and update data sources")]
    DataSource(DataSourceArgs),
    #[command(about = "Inspect, create, and update database containers")]
    Database(DatabaseArgs),
    #[command(about = "Read and mutate Notion blocks")]
    Block(BlockArgs),
    #[command(about = "List, fetch, and create comments")]
    Comment(CommentArgs),
    #[command(about = "Inspect the integration bot and workspace users")]
    User(UserArgs),
    #[command(name = "file-upload")]
    #[command(about = "List, inspect, and upload local files")]
    FileUpload(FileUploadArgs),
}

#[derive(Args)]
#[command(
    about = "Search pages and data sources",
    after_help = "Examples:\n  notioncli search \"roadmap\"\n  notioncli search \"eng\" --type page --limit 5"
)]
struct SearchArgs {
    /// Search text.
    query: String,
    /// Restrict results to pages, data sources, or both.
    #[arg(long, value_enum, default_value_t = SearchKind::All)]
    r#type: SearchKind,
    /// Maximum number of results to return.
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SearchKind {
    All,
    Page,
    DataSource,
}

#[derive(Args)]
#[command(about = "Authentication and profile management")]
struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommand,
}

#[derive(Subcommand)]
enum AuthCommand {
    Login(AuthLoginArgs),
    #[command(about = "List saved profiles and show which one is active")]
    List,
    #[command(about = "Check whether stored credentials are present and currently valid")]
    Doctor,
    Use(AuthUseArgs),
    #[command(about = "Fetch the live Notion user object for the active profile")]
    Whoami,
    Logout(AuthLogoutArgs),
}

#[derive(Args)]
#[command(
    about = "Validate and store a Notion integration token",
    long_about = "Validate and store a Notion integration token.\n\nIf `--token` is omitted, the command prompts once with hidden input, validates the token against Notion, stores it in the local credentials file, and marks the resulting profile as active. Set `NOTION_TOKEN` for one-shot use without saving."
)]
struct AuthLoginArgs {
    /// Optional profile name to store. If omitted, the CLI derives one automatically.
    #[arg(long)]
    profile_name: Option<String>,
    /// Integration token. If omitted, the CLI prompts once with hidden input.
    #[arg(long)]
    token: Option<String>,
}

#[derive(Args)]
#[command(about = "Switch the active saved profile")]
struct AuthUseArgs {
    /// Profile name to activate.
    profile_name: String,
}

#[derive(Args)]
#[command(about = "Delete a saved profile and remove its secret from the local credentials file")]
struct AuthLogoutArgs {
    /// Profile name to remove. Defaults to the active profile.
    profile_name: Option<String>,
}

#[derive(Args)]
#[command(about = "Page operations")]
struct PageArgs {
    #[command(subcommand)]
    command: PageCommand,
}

#[derive(Subcommand)]
enum PageCommand {
    Get(PageGetArgs),
    #[command(about = "Fetch a specific page property, optionally with pagination")]
    Property(PagePropertyArgs),
    Create(PageCreateArgs),
    Append(PageAppendArgs),
    Replace(PageReplaceArgs),
    Update(PageUpdateArgs),
    #[command(about = "Move a page to the trash")]
    Trash(PageTrashArgs),
    #[command(about = "Restore a trashed page")]
    Restore(PageRestoreArgs),
}

#[derive(Args)]
#[command(
    about = "Fetch a page",
    after_help = "Examples:\n  notioncli page get <page-id-or-url>\n  notioncli page get <page-id-or-url> --include-markdown"
)]
struct PageGetArgs {
    /// Page ID or Notion page URL.
    page: String,
    /// Also fetch the Notion page markdown representation.
    #[arg(long)]
    include_markdown: bool,
    /// Include transcript content when requesting markdown, if available.
    #[arg(long)]
    include_transcript: bool,
}

#[derive(Args)]
#[command(about = "Fetch a single page property value")]
struct PagePropertyArgs {
    /// Page ID or Notion page URL.
    page: String,
    /// Property ID from the page schema.
    property: String,
    #[command(flatten)]
    cursor: CursorArgs,
}

#[derive(Args)]
#[command(
    about = "Create a page under a page or data source parent",
    after_help = "Examples:\n  notioncli page create --parent-page <page-id> --title \"New page\"\n  notioncli page create --parent-data-source <data-source-id> --title \"New row\"\n  notioncli page create --parent-page <page-id> --title \"Imported\" --from-file ./page.md"
)]
#[command(group(
    ArgGroup::new("parent_target")
        .args(["parent", "parent_page", "parent_data_source"])
        .required(true)
        .multiple(false)
))]
struct PageCreateArgs {
    /// Auto-detect whether the parent is a page or a data source.
    #[arg(long, group = "parent_target")]
    parent: Option<String>,
    /// Create under a page parent without an extra parent-type lookup.
    #[arg(long, group = "parent_target")]
    parent_page: Option<String>,
    /// Create under a data source parent. Use --title-property to skip a metadata lookup.
    #[arg(long, group = "parent_target")]
    parent_data_source: Option<String>,
    /// Title property name for --parent-data-source. If omitted, the CLI looks it up first.
    #[arg(long, requires = "parent_data_source")]
    title_property: Option<String>,
    /// Title for the new page or data source row.
    #[arg(long)]
    title: String,
    /// Optional markdown file to use as initial page content.
    #[arg(long)]
    from_file: Option<PathBuf>,
    /// Read optional markdown content from stdin.
    #[arg(long)]
    stdin: bool,
}

#[derive(Args)]
#[command(
    about = "Append markdown content to an existing page",
    after_help = "Examples:\n  notioncli page append <page-id> --from-file ./append.md\n  printf '## Notes\\n' | notioncli page append <page-id> --stdin"
)]
struct PageAppendArgs {
    /// Page ID or Notion page URL.
    page: String,
    /// Read markdown content from a file.
    #[arg(long)]
    from_file: Option<PathBuf>,
    /// Read markdown content from stdin.
    #[arg(long)]
    stdin: bool,
}

#[derive(Args)]
#[command(
    about = "Replace a page's markdown content",
    after_help = "Example:\n  notioncli page replace <page-id> --from-file ./replacement.md --allow-deleting-content"
)]
struct PageReplaceArgs {
    /// Page ID or Notion page URL.
    page: String,
    /// Read replacement markdown from a file.
    #[arg(long)]
    from_file: Option<PathBuf>,
    /// Read replacement markdown from stdin.
    #[arg(long)]
    stdin: bool,
    /// Allow Notion to delete existing content while replacing the page.
    #[arg(long)]
    allow_deleting_content: bool,
}

#[derive(Args)]
#[command(
    about = "Update page metadata with a raw JSON request body",
    after_help = "Example:\n  notioncli page update <page-id> --body-json '{\"icon\":{\"type\":\"emoji\",\"emoji\":\"🧪\"}}'"
)]
struct PageUpdateArgs {
    /// Page ID or Notion page URL.
    page: String,
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(about = "Move a page to the trash")]
struct PageTrashArgs {
    /// Page ID or Notion page URL.
    page: String,
}

#[derive(Args)]
#[command(about = "Restore a trashed page")]
struct PageRestoreArgs {
    /// Page ID or Notion page URL.
    page: String,
}

#[derive(Args)]
#[command(about = "Data source operations")]
struct DataSourceArgs {
    #[command(subcommand)]
    command: DataSourceCommand,
}

#[derive(Subcommand)]
enum DataSourceCommand {
    Get(DataSourceGetArgs),
    Query(DataSourceQueryArgs),
    #[command(about = "Create a data source from a raw JSON request body")]
    Create(DataSourceCreateArgs),
    #[command(about = "Update a data source with a raw JSON request body")]
    Update(DataSourceUpdateArgs),
}

#[derive(Args)]
#[command(about = "Fetch a data source")]
struct DataSourceGetArgs {
    /// Data source ID or Notion URL.
    data_source: String,
}

#[derive(Args)]
#[command(
    about = "Query entries in a data source",
    after_help = "Examples:\n  notioncli data-source query <data-source-id>\n  notioncli data-source query <data-source-id> --filter-json '{\"property\":\"Status\",\"status\":{\"equals\":\"Done\"}}'"
)]
struct DataSourceQueryArgs {
    /// Data source ID or Notion URL.
    data_source: String,
    /// Raw JSON filter object accepted by the Notion API.
    #[arg(long)]
    filter_json: Option<String>,
    /// Raw JSON sort array accepted by the Notion API.
    #[arg(long)]
    sort_json: Option<String>,
    /// Maximum number of results to return in this page.
    #[arg(long)]
    page_size: Option<usize>,
    /// Cursor from a previous response.
    #[arg(long)]
    cursor: Option<String>,
}

#[derive(Args, Clone)]
struct CursorArgs {
    /// Maximum number of results to return in this page.
    #[arg(long)]
    page_size: Option<usize>,
    /// Cursor from a previous paginated response.
    #[arg(long)]
    cursor: Option<String>,
}

#[derive(Args, Clone)]
struct JsonInputArgs {
    /// Inline JSON request body.
    #[arg(long)]
    body_json: Option<String>,
    /// Read the JSON request body from a file.
    #[arg(long)]
    from_file: Option<PathBuf>,
    /// Read the JSON request body from stdin.
    #[arg(long)]
    stdin: bool,
}

#[derive(Args)]
#[command(
    about = "Create a data source with a raw JSON request body",
    after_help = "Example:\n  notioncli data-source create --from-file ./data-source.json"
)]
struct DataSourceCreateArgs {
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(
    about = "Update a data source with a raw JSON request body",
    after_help = "Example:\n  notioncli data-source update <data-source-id> --from-file ./patch.json"
)]
struct DataSourceUpdateArgs {
    /// Data source ID or Notion URL.
    data_source: String,
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(about = "Database operations")]
struct DatabaseArgs {
    #[command(subcommand)]
    command: DatabaseCommand,
}

#[derive(Subcommand)]
enum DatabaseCommand {
    Get(DatabaseGetArgs),
    #[command(about = "Create a database with a raw JSON request body")]
    Create(DatabaseCreateArgs),
    #[command(about = "Update a database with a raw JSON request body")]
    Update(DatabaseUpdateArgs),
}

#[derive(Args)]
#[command(about = "Fetch a database")]
struct DatabaseGetArgs {
    /// Database ID or Notion URL.
    database: String,
}

#[derive(Args)]
#[command(
    about = "Create a database with a raw JSON request body",
    after_help = "Example:\n  notioncli database create --from-file ./database.json"
)]
struct DatabaseCreateArgs {
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(
    about = "Update a database with a raw JSON request body",
    after_help = "Example:\n  notioncli database update <database-id> --from-file ./patch.json"
)]
struct DatabaseUpdateArgs {
    /// Database ID or Notion URL.
    database: String,
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(about = "Block operations")]
struct BlockArgs {
    #[command(subcommand)]
    command: BlockCommand,
}

#[derive(Subcommand)]
enum BlockCommand {
    Get(BlockGetArgs),
    Children(BlockChildrenArgs),
    #[command(about = "Append child blocks with a raw JSON request body")]
    Append(BlockAppendArgs),
    #[command(about = "Update a block with a raw JSON request body")]
    Update(BlockUpdateArgs),
    #[command(about = "Delete a block")]
    Delete(BlockDeleteArgs),
}

#[derive(Args)]
#[command(about = "Fetch a block")]
struct BlockGetArgs {
    /// Block ID or Notion URL.
    block: String,
}

#[derive(Args)]
#[command(about = "List child blocks")]
struct BlockChildrenArgs {
    /// Block ID or Notion URL.
    block: String,
    #[command(flatten)]
    cursor: CursorArgs,
}

#[derive(Args)]
#[command(
    about = "Append child blocks with a raw JSON request body",
    after_help = "Example:\n  notioncli block append <block-id> --body-json '{\"children\":[{\"object\":\"block\",\"type\":\"paragraph\",\"paragraph\":{\"rich_text\":[{\"type\":\"text\",\"text\":{\"content\":\"Hello\"}}]}}]}'"
)]
struct BlockAppendArgs {
    /// Block ID or Notion URL.
    block: String,
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(
    about = "Update a block with a raw JSON request body",
    after_help = "Example:\n  notioncli block update <block-id> --body-json '{\"paragraph\":{\"rich_text\":[{\"type\":\"text\",\"text\":{\"content\":\"Updated\"}}]}}'"
)]
struct BlockUpdateArgs {
    /// Block ID or Notion URL.
    block: String,
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(about = "Delete a block")]
struct BlockDeleteArgs {
    /// Block ID or Notion URL.
    block: String,
}

#[derive(Args)]
#[command(about = "Comment operations")]
struct CommentArgs {
    #[command(subcommand)]
    command: CommentCommand,
}

#[derive(Subcommand)]
enum CommentCommand {
    List(CommentListArgs),
    Get(CommentGetArgs),
    #[command(about = "Create a comment with a raw JSON request body")]
    Create(CommentCreateArgs),
}

#[derive(Args)]
#[command(about = "List comments for a page or block")]
struct CommentListArgs {
    /// Page ID, block ID, or Notion URL.
    target: String,
    #[command(flatten)]
    cursor: CursorArgs,
}

#[derive(Args)]
#[command(about = "Fetch a single comment")]
struct CommentGetArgs {
    /// Comment ID.
    comment: String,
}

#[derive(Args)]
#[command(
    about = "Create a comment with a raw JSON request body",
    after_help = "Example:\n  notioncli comment create --from-file ./comment.json"
)]
struct CommentCreateArgs {
    #[command(flatten)]
    body: JsonInputArgs,
}

#[derive(Args)]
#[command(about = "User and bot identity operations")]
struct UserArgs {
    #[command(subcommand)]
    command: UserCommand,
}

#[derive(Subcommand)]
enum UserCommand {
    #[command(about = "Fetch the current integration bot user")]
    Me,
    List(UserListArgs),
    Get(UserGetArgs),
}

#[derive(Args)]
#[command(about = "List users in the workspace")]
struct UserListArgs {
    #[command(flatten)]
    cursor: CursorArgs,
}

#[derive(Args)]
#[command(about = "Fetch a specific user")]
struct UserGetArgs {
    /// User ID.
    user: String,
}

#[derive(Args)]
#[command(about = "File upload operations")]
struct FileUploadArgs {
    #[command(subcommand)]
    command: FileUploadCommand,
}

#[derive(Subcommand)]
enum FileUploadCommand {
    List(FileUploadListArgs),
    Get(FileUploadGetArgs),
    #[command(about = "Upload a local file with the direct single-part flow")]
    Create(FileUploadCreateArgs),
}

#[derive(Args)]
#[command(about = "List file uploads")]
struct FileUploadListArgs {
    /// Optional file-upload status filter.
    #[arg(long)]
    status: Option<String>,
    #[command(flatten)]
    cursor: CursorArgs,
}

#[derive(Args)]
#[command(about = "Fetch a specific file upload")]
struct FileUploadGetArgs {
    /// File upload ID.
    file_upload: String,
}

#[derive(Args)]
#[command(
    about = "Upload a local file with the direct single-part flow",
    after_help = "Example:\n  notioncli file-upload create --file ./image.png --content-type image/png"
)]
struct FileUploadCreateArgs {
    /// Local file path to upload.
    #[arg(long)]
    file: PathBuf,
    /// Optional MIME type. If omitted, Notion infers it from the filename.
    #[arg(long)]
    content_type: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => match error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                let _ = error.print();
                process::exit(0);
            }
            _ => {
                let output = requested_output_format_from_argv();
                let _ = output.print_error(400, "invalid_request", &error.to_string());
                process::exit(2);
            }
        },
    };

    let output = cli.output;
    if let Err(error) = run(cli).await {
        let classified = classify_error(&error);
        let _ = output.print_error(classified.status, classified.code, &error.to_string());
        process::exit(classified.exit_code);
    }
}

fn requested_output_format_from_argv() -> OutputFormat {
    let args: Vec<OsString> = std::env::args_os().collect();
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--output" {
            if let Some(value) = iter.next() {
                if let Some(format) = parse_output_format(value.to_string_lossy().as_ref()) {
                    return format;
                }
            }
            continue;
        }

        if let Some(value) = arg.to_string_lossy().strip_prefix("--output=") {
            if let Some(format) = parse_output_format(value) {
                return format;
            }
        }
    }

    OutputFormat::Human
}

fn parse_output_format(value: &str) -> Option<OutputFormat> {
    match value {
        "human" => Some(OutputFormat::Human),
        "json" => Some(OutputFormat::Json),
        "yaml" => Some(OutputFormat::Yaml),
        _ => None,
    }
}

async fn run(cli: Cli) -> Result<()> {
    init_tracing(cli.verbose)?;

    let mut store = ConfigStore::load()?;
    let client = NotionClient::from_config(&store)?;

    let globals = GlobalOptions {
        profile: cli.profile.clone(),
        output: cli.output,
    };
    let command = cli.command;

    match command {
        Commands::Auth(args) => handle_auth(args, &globals, &client, &mut store).await,
        Commands::Search(args) => handle_search(args, &globals, &client, &mut store).await,
        Commands::Page(args) => handle_page(args, &globals, &client, &mut store).await,
        Commands::DataSource(args) => handle_data_source(args, &globals, &client, &mut store).await,
        Commands::Database(args) => handle_database(args, &globals, &client, &mut store).await,
        Commands::Block(args) => handle_block(args, &globals, &client, &mut store).await,
        Commands::Comment(args) => handle_comment(args, &globals, &client, &mut store).await,
        Commands::User(args) => handle_user(args, &globals, &client, &mut store).await,
        Commands::FileUpload(args) => handle_file_upload(args, &globals, &client, &mut store).await,
    }
}

fn init_tracing(verbose: u8) -> Result<()> {
    let filter = match verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow!("failed to initialize logging: {error}"))
}

struct ClassifiedError {
    status: u16,
    code: &'static str,
    exit_code: i32,
}

fn classify_error(error: &anyhow::Error) -> ClassifiedError {
    let message = error.to_string().to_ascii_lowercase();

    if message.contains("rate limit") || message.contains("429") || message.contains("retry-after")
    {
        return ClassifiedError {
            status: 429,
            code: "rate_limited",
            exit_code: 7,
        };
    }

    if message.contains("authentication failed")
        || message.contains("no active profile")
        || message.contains("provide a notion integration token")
        || message.contains("failed to read token input")
    {
        return ClassifiedError {
            status: 401,
            code: "unauthorized",
            exit_code: 3,
        };
    }

    if message.contains("could not find that object")
        || message.contains("does not look like a notion page or data source")
    {
        return ClassifiedError {
            status: 404,
            code: "object_not_found",
            exit_code: 5,
        };
    }

    if message.contains("validation error")
        || message.contains("rejected the request as invalid")
        || message.contains("failed to parse --filter-json")
        || message.contains("failed to parse --sort-json")
        || message.contains("failed to parse json request body")
        || message.contains("provide json body with --body-json, --from-file, or --stdin")
        || message.contains("use only one of --body-json, --from-file, or --stdin")
        || message.contains("json request body must include `children`")
        || message.contains("use either --from-file or --stdin")
        || message.contains("provide markdown with --from-file or --stdin")
    {
        return ClassifiedError {
            status: 400,
            code: "validation_error",
            exit_code: 6,
        };
    }

    if message.contains("config")
        || message.contains("credentials file")
        || message.contains("failed to write temporary credentials file")
        || message.contains("failed to replace credentials file")
        || message.contains("failed to read file")
        || message.contains("failed to read stdin")
    {
        return ClassifiedError {
            status: 400,
            code: "invalid_request",
            exit_code: 4,
        };
    }

    if message.contains("rejected the request with 403") {
        return ClassifiedError {
            status: 403,
            code: "forbidden",
            exit_code: 9,
        };
    }

    if message.contains("notion api error") || message.contains("request to notion failed") {
        return ClassifiedError {
            status: 500,
            code: "internal_server_error",
            exit_code: 8,
        };
    }

    ClassifiedError {
        status: 500,
        code: "internal_server_error",
        exit_code: 10,
    }
}

async fn handle_auth(
    args: AuthArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    match args.command {
        AuthCommand::Login(login) => login_internal(login, globals, client, store).await,
        AuthCommand::List => {
            let profiles = store
                .profiles()
                .iter()
                .map(|(name, meta)| {
                    json!({
                        "name": name,
                        "active": store.active_profile() == Some(name.as_str()),
                        "profile": meta,
                    })
                })
                .collect::<Vec<_>>();
            globals
                .output
                .print_success(&json!({ "profiles": profiles }))
        }
        AuthCommand::Doctor => auth_doctor(globals, client, store).await,
        AuthCommand::Use(args) => {
            store.set_active_profile(&args.profile_name)?;
            globals
                .output
                .print_success(&json!({ "active_profile": args.profile_name }))
        }
        AuthCommand::Whoami => {
            let mut session = store.resolve_session(globals.profile.as_deref())?;
            let live = client.get_self(&mut session, store).await?;
            let active_profile = session.profile_name.clone();
            let stored_meta = active_profile
                .as_deref()
                .and_then(|name| store.get_profile(name))
                .cloned();

            let payload = json!({
                "profile": active_profile.unwrap_or_else(|| session.display_name().to_string()),
                "stored_profile": stored_meta,
                "live_user": live,
                "live_object": object_kind(&live),
            });
            globals.output.print_success(&payload)
        }
        AuthCommand::Logout(args) => {
            let profile_name = args
                .profile_name
                .or_else(|| globals.profile.clone())
                .or_else(|| store.active_profile().map(str::to_string))
                .ok_or_else(|| anyhow!("no profile specified and there is no active profile"))?;
            store.remove_profile(&profile_name)?;
            globals
                .output
                .print_success(&json!({ "deleted_profile": profile_name }))
        }
    }
}

async fn auth_doctor(
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let active_profile = globals
        .profile
        .clone()
        .or_else(|| store.active_profile().map(str::to_string));
    let session_result = store.resolve_session(active_profile.as_deref());

    let stored_credentials = active_profile
        .as_deref()
        .map(|name| store.has_persisted_secret(name))
        .unwrap_or(false);

    let (auth_ok, auth_error, live_user, token_source, token_persistence) = match session_result {
        Ok(mut session) => {
            let source = session.source.as_str();
            match client.get_self(&mut session, store).await {
                Ok(user) => (
                    true,
                    Option::<String>::None,
                    Some(user),
                    Some(source),
                    Some(source),
                ),
                Err(error) => (
                    false,
                    Some(error.to_string()),
                    None,
                    Some(source),
                    Some(source),
                ),
            }
        }
        Err(error) => (false, Some(error.to_string()), None, None, None),
    };

    globals.output.print_success(&json!({
        "active_profile": active_profile,
        "stored_credentials": stored_credentials,
        "auth_ok": auth_ok,
        "auth_error": auth_error,
        "live_user": live_user,
        "token_source": token_source,
        "token_persistence": token_persistence,
        "login_flow": "run `notioncli auth login` to save a token locally, or set `NOTION_TOKEN` for one-shot use",
    }))
}

async fn login_internal(
    args: AuthLoginArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let token = match args.token {
        Some(token) if !token.trim().is_empty() => token,
        Some(_) | None => prompt_for_internal_token()?,
    };

    let me = client.get_self_for_token(&token).await?;
    let meta = ProfileMeta {
        auth_type: AuthType::Internal,
        workspace_id: None,
        workspace_name: None,
        bot_id: bot_id_from_user_me(&me),
        owner_name: owner_name_from_user_me(&me),
        owner_email: owner_email_from_user_me(&me),
    };

    let profile_name = args
        .profile_name
        .unwrap_or_else(|| derive_profile_name(&meta));
    let secret = StoredSecret::Internal { token };
    store.put_profile(profile_name.clone(), meta.clone(), &secret)?;

    emit_auth_login_result(globals, &profile_name, &meta)
}

fn emit_auth_login_result(
    globals: &GlobalOptions,
    profile_name: &str,
    meta: &ProfileMeta,
) -> Result<()> {
    globals.output.print_success(&json!({
        "profile_name": profile_name,
        "profile": meta,
    }))
}

async fn handle_search(
    args: SearchArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;
    let filter = match args.r#type {
        SearchKind::All => SearchFilter::All,
        SearchKind::Page => SearchFilter::Page,
        SearchKind::DataSource => SearchFilter::DataSource,
    };
    let response = client
        .search(&mut session, store, &args.query, filter, args.limit)
        .await?;
    globals.output.print_success(&response)
}

async fn handle_page(
    args: PageArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;

    match args.command {
        PageCommand::Get(args) => {
            let page = client.get_page(&mut session, store, &args.page).await?;
            let payload = if args.include_markdown {
                let markdown = client
                    .get_page_markdown(&mut session, store, &args.page, args.include_transcript)
                    .await?;
                json!({
                    "page": page,
                    "markdown": markdown,
                })
            } else {
                page
            };
            globals.output.print_success(&payload)
        }
        PageCommand::Property(args) => {
            let property = client
                .get_page_property(
                    &mut session,
                    store,
                    &args.page,
                    &args.property,
                    args.cursor.page_size,
                    args.cursor.cursor,
                )
                .await?;
            globals.output.print_success(&property)
        }
        PageCommand::Create(args) => {
            let markdown = read_optional_markdown(args.from_file, args.stdin)?;
            let parent = if let Some(parent) = args.parent {
                client
                    .resolve_create_parent(&mut session, store, &parent)
                    .await?
            } else if let Some(parent_page) = args.parent_page {
                CreateParent::Page {
                    page_id: normalize_notion_id(&parent_page)?,
                }
            } else if let Some(parent_data_source) = args.parent_data_source {
                client
                    .resolve_data_source_parent(
                        &mut session,
                        store,
                        &parent_data_source,
                        args.title_property,
                    )
                    .await?
            } else {
                unreachable!("clap requires exactly one parent target");
            };
            let page = client
                .create_page(&mut session, store, parent, &args.title, markdown)
                .await?;
            globals.output.print_success(&page)
        }
        PageCommand::Append(args) => {
            let markdown = read_required_markdown(args.from_file, args.stdin)?;
            let response = client
                .append_page_markdown(&mut session, store, &args.page, markdown)
                .await?;
            globals.output.print_success(&response)
        }
        PageCommand::Replace(args) => {
            let markdown = read_required_markdown(args.from_file, args.stdin)?;
            let response = client
                .replace_page_markdown(
                    &mut session,
                    store,
                    &args.page,
                    markdown,
                    args.allow_deleting_content,
                )
                .await?;
            globals.output.print_success(&response)
        }
        PageCommand::Update(args) => {
            let body = read_required_json_input(&args.body)?;
            let page = client
                .update_page(&mut session, store, &args.page, body)
                .await?;
            globals.output.print_success(&page)
        }
        PageCommand::Trash(args) => {
            let page = client
                .update_page(&mut session, store, &args.page, json!({ "in_trash": true }))
                .await?;
            globals.output.print_success(&page)
        }
        PageCommand::Restore(args) => {
            let page = client
                .update_page(
                    &mut session,
                    store,
                    &args.page,
                    json!({ "in_trash": false }),
                )
                .await?;
            globals.output.print_success(&page)
        }
    }
}

async fn handle_data_source(
    args: DataSourceArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;

    match args.command {
        DataSourceCommand::Get(args) => {
            let data_source = client
                .get_data_source(&mut session, store, &args.data_source)
                .await?;
            globals.output.print_success(&data_source)
        }
        DataSourceCommand::Query(args) => {
            let filter = parse_json_arg(args.filter_json.as_deref(), "--filter-json")?;
            let sorts = parse_json_arg(args.sort_json.as_deref(), "--sort-json")?;
            let response = client
                .query_data_source(
                    &mut session,
                    store,
                    &args.data_source,
                    filter,
                    sorts,
                    args.page_size,
                    args.cursor,
                )
                .await?;
            globals.output.print_success(&response)
        }
        DataSourceCommand::Create(args) => {
            let body = read_required_json_input(&args.body)?;
            let data_source = client.create_data_source(&mut session, store, body).await?;
            globals.output.print_success(&data_source)
        }
        DataSourceCommand::Update(args) => {
            let body = read_required_json_input(&args.body)?;
            let data_source = client
                .update_data_source(&mut session, store, &args.data_source, body)
                .await?;
            globals.output.print_success(&data_source)
        }
    }
}

async fn handle_database(
    args: DatabaseArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;

    match args.command {
        DatabaseCommand::Get(args) => {
            let database = client
                .get_database(&mut session, store, &args.database)
                .await?;
            globals.output.print_success(&database)
        }
        DatabaseCommand::Create(args) => {
            let body = read_required_json_input(&args.body)?;
            let database = client.create_database(&mut session, store, body).await?;
            globals.output.print_success(&database)
        }
        DatabaseCommand::Update(args) => {
            let body = read_required_json_input(&args.body)?;
            let database = client
                .update_database(&mut session, store, &args.database, body)
                .await?;
            globals.output.print_success(&database)
        }
    }
}

async fn handle_block(
    args: BlockArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;

    match args.command {
        BlockCommand::Get(args) => {
            let block = client.get_block(&mut session, store, &args.block).await?;
            globals.output.print_success(&block)
        }
        BlockCommand::Children(args) => {
            let response = client
                .get_block_children(
                    &mut session,
                    store,
                    &args.block,
                    args.cursor.page_size,
                    args.cursor.cursor,
                )
                .await?;
            globals.output.print_success(&response)
        }
        BlockCommand::Append(args) => {
            let body = read_required_json_input(&args.body)?;
            let children = body
                .get("children")
                .cloned()
                .ok_or_else(|| anyhow!("JSON request body must include `children`"))?;
            let position = body.get("position").cloned();
            let result = client
                .append_block_children(&mut session, store, &args.block, children, position)
                .await?;
            globals.output.print_success(&result)
        }
        BlockCommand::Update(args) => {
            let body = read_required_json_input(&args.body)?;
            let block = client
                .update_block(&mut session, store, &args.block, body)
                .await?;
            globals.output.print_success(&block)
        }
        BlockCommand::Delete(args) => {
            let block = client
                .delete_block(&mut session, store, &args.block)
                .await?;
            globals.output.print_success(&block)
        }
    }
}

async fn handle_comment(
    args: CommentArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;

    match args.command {
        CommentCommand::List(args) => {
            let response = client
                .list_comments(
                    &mut session,
                    store,
                    &args.target,
                    args.cursor.page_size,
                    args.cursor.cursor,
                )
                .await?;
            globals.output.print_success(&response)
        }
        CommentCommand::Get(args) => {
            let comment = client
                .get_comment(&mut session, store, &args.comment)
                .await?;
            globals.output.print_success(&comment)
        }
        CommentCommand::Create(args) => {
            let body = read_required_json_input(&args.body)?;
            let comment = client.create_comment(&mut session, store, body).await?;
            globals.output.print_success(&comment)
        }
    }
}

async fn handle_user(
    args: UserArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;

    match args.command {
        UserCommand::Me => {
            let user = client.get_self(&mut session, store).await?;
            globals.output.print_success(&user)
        }
        UserCommand::List(args) => {
            let response = client
                .list_users(
                    &mut session,
                    store,
                    args.cursor.page_size,
                    args.cursor.cursor,
                )
                .await?;
            globals.output.print_success(&response)
        }
        UserCommand::Get(args) => {
            let user = client.get_user(&mut session, store, &args.user).await?;
            globals.output.print_success(&user)
        }
    }
}

async fn handle_file_upload(
    args: FileUploadArgs,
    globals: &GlobalOptions,
    client: &NotionClient,
    store: &mut ConfigStore,
) -> Result<()> {
    let mut session = store.resolve_session(globals.profile.as_deref())?;

    match args.command {
        FileUploadCommand::List(args) => {
            let response = client
                .list_file_uploads(
                    &mut session,
                    store,
                    args.status.as_deref(),
                    args.cursor.page_size,
                    args.cursor.cursor,
                )
                .await?;
            globals.output.print_success(&response)
        }
        FileUploadCommand::Get(args) => {
            let file_upload = client
                .get_file_upload(&mut session, store, &args.file_upload)
                .await?;
            globals.output.print_success(&file_upload)
        }
        FileUploadCommand::Create(args) => {
            const MAX_UPLOAD_SIZE: u64 = 5 * 1024 * 1024; // 5 MB Notion single-part limit
            let file_size = fs::metadata(&args.file)
                .with_context(|| format!("failed to read file {}", args.file.display()))?
                .len();
            if file_size > MAX_UPLOAD_SIZE {
                bail!(
                    "file {} is {} bytes, which exceeds the 5 MB single-part upload limit",
                    args.file.display(),
                    file_size
                );
            }
            let bytes = fs::read(&args.file)
                .with_context(|| format!("failed to read file {}", args.file.display()))?;
            let filename = args
                .file
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| anyhow!("failed to determine the file name for upload"))?;
            let file_upload = client
                .upload_small_file(
                    &mut session,
                    store,
                    filename,
                    args.content_type.as_deref(),
                    bytes,
                )
                .await?;
            globals.output.print_success(&file_upload)
        }
    }
}

fn parse_json_arg(raw: Option<&str>, flag_name: &str) -> Result<Option<Value>> {
    raw.map(|value| {
        serde_json::from_str(value).with_context(|| format!("failed to parse {flag_name} as JSON"))
    })
    .transpose()
}

fn read_optional_json_input(args: &JsonInputArgs) -> Result<Option<Value>> {
    match (&args.body_json, &args.from_file, args.stdin) {
        (Some(_), Some(_), _) | (Some(_), _, true) | (None, Some(_), true) => {
            bail!("use only one of --body-json, --from-file, or --stdin")
        }
        (None, None, false) => Ok(None),
        (Some(body), None, false) => parse_json_text(body, "--body-json").map(Some),
        (None, Some(path), false) => {
            parse_json_text(&read_text_file(path)?, &path.display().to_string()).map(Some)
        }
        (None, None, true) => parse_json_text(&read_stdin_to_string()?, "stdin").map(Some),
    }
}

fn read_required_json_input(args: &JsonInputArgs) -> Result<Value> {
    read_optional_json_input(args)?
        .ok_or_else(|| anyhow!("provide JSON body with --body-json, --from-file, or --stdin"))
}

fn parse_json_text(raw: &str, source: &str) -> Result<Value> {
    serde_json::from_str(raw)
        .with_context(|| format!("failed to parse JSON request body from {source}"))
}

fn read_optional_markdown(from_file: Option<PathBuf>, stdin: bool) -> Result<Option<String>> {
    match (from_file, stdin) {
        (Some(_), true) => bail!("use either --from-file or --stdin, not both"),
        (None, false) => Ok(None),
        (Some(path), false) => Ok(Some(read_text_file(&path)?)),
        (None, true) => Ok(Some(read_stdin_to_string()?)),
    }
}

fn read_required_markdown(from_file: Option<PathBuf>, stdin: bool) -> Result<String> {
    read_optional_markdown(from_file, stdin)?
        .ok_or_else(|| anyhow!("provide markdown with --from-file or --stdin"))
}

fn read_stdin_to_string() -> Result<String> {
    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .context("failed to read stdin")?;
    Ok(buffer)
}

fn prompt_for_internal_token() -> Result<String> {
    let token = prompt_password("Paste your Notion integration token: ")
        .context("failed to read token input")?;
    let token = token.trim().to_string();
    if token.is_empty() {
        bail!("provide a Notion integration token")
    }
    Ok(token)
}

fn derive_profile_name(meta: &ProfileMeta) -> String {
    let seed = meta
        .workspace_name
        .as_deref()
        .or(meta.owner_name.as_deref())
        .or(meta.bot_id.as_deref())
        .unwrap_or("internal");
    let slug = slugify_profile_name(seed);
    if slug.is_empty() {
        "internal".into()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_profile_names() {
        let meta = ProfileMeta {
            auth_type: AuthType::Internal,
            workspace_id: None,
            workspace_name: Some("My Team".into()),
            bot_id: None,
            owner_name: None,
            owner_email: None,
        };
        assert_eq!(derive_profile_name(&meta), "my-team");
    }

    #[test]
    fn reads_inline_json_input() -> Result<()> {
        let args = JsonInputArgs {
            body_json: Some(r#"{"hello":"world"}"#.into()),
            from_file: None,
            stdin: false,
        };

        let value = read_required_json_input(&args)?;
        assert_eq!(value.get("hello").and_then(Value::as_str), Some("world"));
        Ok(())
    }

    #[test]
    fn rejects_multiple_json_input_sources() {
        let args = JsonInputArgs {
            body_json: Some("{}".into()),
            from_file: Some(PathBuf::from("body.json")),
            stdin: false,
        };

        let error = read_optional_json_input(&args).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("use only one of --body-json, --from-file, or --stdin")
        );
    }
}
