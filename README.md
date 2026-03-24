# notioncli

A Rust CLI for the official Notion API.

The default output is human-friendly and readable in a terminal. If you want a
machine-stable format, use `--output json` or `--output yaml`.

Authentication is local-first:

- use a Notion internal integration token
- run `auth login` once
- the token is validated and stored in a local credentials file
- future commands reuse the active profile automatically

There is no backend and no OAuth dependency in the normal workflow. You can
also set `NOTION_TOKEN` for one-shot use without saving a profile.

## Install

Tagged releases publish prebuilt binaries to GitHub Releases.

From GitHub Releases:

1. Open the repo's Releases page.
2. Download the archive for your platform.
3. Extract it.
4. Move `notioncli` somewhere on your `PATH`, such as `~/.local/bin` on
   macOS/Linux.

From source:

```bash
cargo install --path .
```

That installs the `notioncli` binary into Cargo's bin directory.

For local development:

```bash
cargo build --release
./target/release/notioncli --help
```

## Quickstart

1. Create a Notion internal integration in the Notion integrations dashboard.
2. Enable the capabilities you need.
3. Share the target pages and data sources with that integration.
4. Log in once:

```bash
notioncli auth login
```

5. Verify the saved credential:

```bash
notioncli auth doctor
notioncli auth whoami
```

6. Try a few real commands:

```bash
notioncli search "roadmap"
notioncli page get "<page-id-or-url>" --include-markdown
notioncli page create --parent-page "<page-id>" --title "New page"
```

## What It Supports

- Interactive internal-integration login with one-time token entry
- File-backed credential storage with named profiles
- Search across pages and data sources
- Read pages as structured data, optionally including enhanced markdown
- Create, update, trash, and restore pages
- Read page properties
- Append or replace page markdown
- Retrieve, append, update, and delete blocks
- Inspect and query data sources
- Retrieve, create, and update databases and data sources
- List and retrieve users
- List, retrieve, and create comments
- List, retrieve, and upload files with direct single-part upload

API coverage details, including partial and intentionally unsupported areas, are
documented in [`API_COVERAGE.md`](./API_COVERAGE.md).
Benchmark snapshots are documented in [`BENCHMARKS.md`](./BENCHMARKS.md).

## Live Testing

The repo includes two live test scripts for a real Notion workspace:

- `scripts/live_smoke.sh`
  - fast auth, output, page, and block smoke checks
- `scripts/live_matrix.sh`
  - broader pass/fail matrix across auth, users, pages, blocks, comments,
    databases, data sources, file uploads, and expected failure modes

Requirements:

- a logged-in profile via `notioncli auth login`
- `jq`
- a shared parent page, or a workspace where `search "" --type page --limit 1`
  can discover one automatically

Examples:

```bash
scripts/live_smoke.sh
NOTION_TEST_PARENT_ID="<page-id>" scripts/live_matrix.sh
NOTION_TEST_PARENT_ID="<page-id>" NOTION_TEST_KEEP=1 scripts/live_matrix.sh
NOTIONCLI_BIN=./target/release/notioncli scripts/live_matrix.sh
```

Notes:

- the matrix creates disposable pages, comments, uploads, databases, and data
  sources under the chosen parent page
- by default the matrix trashes its container page at the end; set
  `NOTION_TEST_KEEP=1` to keep the artifacts
- `auth logout` is intentionally not exercised by the live scripts because it
  deletes the saved local credential

## Benchmarking vs Notion MCP

The repo also includes a separate benchmark path for comparing this CLI against
Notion's hosted MCP server:

Important: the checked-in benchmark snapshot is a one-shot tool benchmark. It
compares:

- CLI as a fresh process per call
- MCP as a fresh session per call

It does not represent reused-session MCP performance, and it does not represent
a hypothetical persistent long-lived CLI mode.

- `scripts/notion_mcp_login.py`
  - runs the hosted MCP OAuth flow and stores a local token bundle
- `scripts/mcp_list_tools.py`
  - lists the exposed MCP tools and optionally their input schemas
- `scripts/bench_case_once.py`
  - runs one CLI or MCP benchmark case as a single external command
- `scripts/bench_prepare_comparable.py`
  - creates a disposable page/database fixture set for the full comparable suite
- `scripts/bench_cleanup_comparable.py`
  - trashes the disposable container page after the benchmark
- `scripts/bench_strengthen.py`
  - runs repeated read and write suites with stricter fixture isolation and
    session aggregation
- `scripts/hyperfine_compare.sh`
  - uses `hyperfine` to compare process-per-call CLI latency vs one-shot MCP latency
- `scripts/bench_compare.py`
  - benchmarks equivalent CLI commands and MCP tool calls from a JSON case file
- `scripts/bench_cases.sample.json`
  - sample benchmark case definitions with env-var placeholders
- `scripts/bench_cases.comparable.json`
  - expanded overlap suite covering all currently comparable CLI and MCP surfaces
- `scripts/bench_cases.readsuite.json`
  - read-only overlap suite for the strengthened benchmark
- `scripts/bench_cases.writesuite.json`
  - write overlap suite for the strengthened benchmark

Suggested flow:

```bash
python3 scripts/notion_mcp_login.py
python3 scripts/mcp_list_tools.py --schemas
cp scripts/bench_cases.sample.json /tmp/bench_cases.json
PAGE_ID="<page-id>" python3 scripts/bench_compare.py \
  --case-file /tmp/bench_cases.json \
  --runs 20 \
  --warmup 3
```

If you want an external-command benchmark runner instead of the built-in Python
timer, use `hyperfine`:

```bash
brew install hyperfine
eval "$(python3 scripts/bench_prepare_comparable.py)"
CASE_FILE=scripts/bench_cases.comparable.json \
  scripts/hyperfine_compare.sh
python3 scripts/bench_cleanup_comparable.py
```

For the stronger checked-in snapshot, use the orchestrator that splits reads
from writes and isolates mutating cases:

```bash
python3 scripts/bench_strengthen.py \
  --root-parent-id "<page-id>"
```

Notes:

- `scripts/mcp_list_tools.py --schemas` is the source of truth for the hosted
  MCP tool argument shapes; update the sample case file to match those schemas
- `scripts/bench_prepare_comparable.py` prints `export ...` lines; use `eval "$(...)"` to load the fixture ids into your shell
- `scripts/hyperfine_compare.sh` compares:
  - CLI as a fresh process for each run
  - MCP as a fresh session for each run
  - this is the cleanest apples-to-apples process benchmark
- `scripts/bench_strengthen.py` writes aggregated summaries to:
  - `tmp/bench-strengthened/read/summary.json`
  - `tmp/bench-strengthened/write/summary.json`
- `scripts/bench_compare.py` reports CLI process timings and MCP timings in two
  modes:
  - `reused_session`: MCP session initialized once and reused across runs
  - `fresh_session`: MCP session initialized separately for each run
- `NOTION_MCP_ACCESS_TOKEN` can override the stored MCP token file
- the benchmark script expands environment variables inside the case file, so
  placeholders like `$PAGE_ID` are supported

## Requirements

- Rust toolchain
- A Notion integration with the capabilities you need
- A Notion internal integration token

## Build

```bash
cargo build
```

If you are running from the repo without installing the binary, replace
`notioncli ...` in the examples below with `cargo run -- ...`.

## Authentication

Log in and paste your token when prompted:

```bash
notioncli auth login
```

Pass the token directly for automation:

```bash
notioncli auth login --token "secret_xxx"
```

One-shot use without saving anything locally:

```bash
NOTION_TOKEN="secret_xxx" notioncli search "roadmap"
```

If you used an older build that stored tokens in the OS keychain, run
`notioncli auth login` again once to populate the local credentials file.

List profiles:

```bash
notioncli auth list
```

Check whether credentials are stored and whether the current profile can reach Notion:

```bash
notioncli auth doctor
```

Switch the active profile:

```bash
notioncli auth use my-workspace
```

## Usage

Search:

```bash
notioncli search "roadmap" --type all --limit 20
```

Read a page:

```bash
notioncli page get "https://www.notion.so/...your-page-url..."
```

Read a page and include markdown:

```bash
notioncli page get "<page-id-or-url>" --include-markdown
```

Create a page from a markdown file:

```bash
notioncli page create \
  --parent-page "<page-id>" \
  --title "CLI-created page" \
  --from-file ./notes.md
```

Create a row under a data source:

```bash
notioncli page create \
  --parent-data-source "<data-source-id>" \
  --title "CLI-created row"
```

If you already know the title property for the data source, pass it to avoid
the metadata lookup:

```bash
notioncli page create \
  --parent-data-source "<data-source-id>" \
  --title-property "Name" \
  --title "CLI-created row"
```

Append markdown from stdin:

```bash
printf "## Added from the CLI\n" | notioncli page append "<page-id>" --stdin
```

Replace page content:

```bash
notioncli page replace "<page-id>" --from-file ./replacement.md --allow-deleting-content
```

Read a page property:

```bash
notioncli page property "<page-id>" "<property-id>"
```

Update a page with a raw JSON body:

```bash
notioncli page update "<page-id>" \
  --body-json '{"cover":{"type":"external","external":{"url":"https://example.com/cover.png"}}}'
```

Trash or restore a page:

```bash
notioncli page trash "<page-id>"
notioncli page restore "<page-id>"
```

Read and update blocks:

```bash
notioncli block get "<block-id>"
notioncli block children "<block-id>"
notioncli block update "<block-id>" \
  --body-json '{"paragraph":{"rich_text":[{"type":"text","text":{"content":"Updated"}}]}}'
```

Append child blocks:

```bash
notioncli block append "<block-id>" \
  --body-json '{"children":[{"object":"block","type":"paragraph","paragraph":{"rich_text":[{"type":"text","text":{"content":"Hello from block append"}}]}}]}'
```

Inspect a data source:

```bash
notioncli data-source get "<data-source-id>"
```

Query a data source:

```bash
notioncli data-source query "<data-source-id>" \
  --filter-json '{"property":"Status","status":{"equals":"In progress"}}' \
  --sort-json '[{"property":"Priority","direction":"descending"}]'
```

Create or update a database or data source with raw JSON:

```bash
notioncli database create --from-file ./database.json
notioncli database update "<database-id>" --from-file ./database-patch.json
notioncli data-source create --from-file ./data-source.json
notioncli data-source update "<data-source-id>" --from-file ./data-source-patch.json
```

Users and comments:

```bash
notioncli user me
notioncli user list
notioncli comment list "<page-or-block-id>"
notioncli comment create --from-file ./comment.json
```

Direct single-part file upload:

```bash
notioncli file-upload create --file ./image.png --content-type image/png
```

Output formats:

```bash
notioncli search "roadmap"
notioncli --output json search "roadmap"
notioncli --output yaml data-source get "<data-source-id>"
```

Endpoint commands usually emit the raw Notion response body at the top level.
CLI-only commands such as `auth doctor` emit structured CLI-native objects.

Success output shape for a 1:1 endpoint command:

```json
{
  "object": "page",
  "id": "32cee67e-2a2c-81db-82e0-c03c093680ac",
  "...": "..."
}
```

Error output shape:

```json
{
  "object": "error",
  "status": 400,
  "code": "validation_error",
  "message": "provide markdown with --from-file or --stdin"
}
```

## Notes

- The CLI pins the `Notion-Version` header to `2026-03-11` by default.
- Stored profiles are kept in the platform config directory.
- Persisted tokens are stored in a local credentials file in that config directory.
- Older keychain-backed installs need one fresh `notioncli auth login` to migrate to the file-backed model.
- The recommended flow is: create an internal integration in Notion, copy the token once, and let the CLI store it locally.
- `NOTION_TOKEN` overrides saved credentials for one-shot use.
- Human-readable pretty output is the default. Use `--output json` for compact JSON or `--output yaml` for YAML.
- For automation, call the installed or built binary directly and pass `--output json`.
- This is a local CLI for the official REST API. It is not trying to be a hosted multi-user auth product.
