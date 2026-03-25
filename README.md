# notioncli

A Rust CLI for the official Notion API. Fast, local-first, no backend.

Default output is human-readable. Use `--output json` or `--output yaml` for
machine-stable formats.

## Install

Pick **one** of the following:

| Method | Command / Steps |
|---|---|
| **Homebrew** (macOS / Linux) | `brew install donutdaniel/tap/notioncli` |
| **Cargo** (requires [Rust toolchain](https://rustup.rs)) | `cargo install --path .` |
| **Binary** ([GitHub Releases](https://github.com/donutdaniel/notioncli/releases)) | Download the archive for your platform, extract it, and move `notioncli` somewhere on your `PATH` (e.g. `~/.local/bin`) |

For local development, build without installing:

```bash
cargo build --release
./target/release/notioncli --help
```

Replace `notioncli` with `cargo run --` if running from the repo without
installing.

## Quickstart

1. Create a [Notion internal integration](https://www.notion.so/profile/integrations) and enable the capabilities you need.
2. Share your target pages/databases with the integration.
3. Log in:

```bash
notioncli auth login
# or non-interactively:
notioncli auth login --token "secret_xxx"
```

4. Verify:

```bash
notioncli auth doctor
notioncli auth whoami
```

5. Try it:

```bash
notioncli search "roadmap"
notioncli page get "<page-id-or-url>" --include-markdown
notioncli page create --parent-page "<page-id>" --title "New page"
```

## Global Flags

| Flag | Description |
|---|---|
| `--output <format>` | `human` (default), `json`, or `yaml` |
| `--profile <name>` | Use a specific saved profile instead of the active one |
| `--verbose` | Increase log verbosity. Repeat (`--verbose --verbose`) for more detail |

## Commands

### auth

```bash
notioncli auth login                      # interactive token prompt
notioncli auth login --token "secret_xxx" # non-interactive
notioncli auth login --profile-name work  # name the profile explicitly
notioncli auth list                       # list saved profiles
notioncli auth doctor                     # check stored credentials and API reachability
notioncli auth whoami                     # fetch the live Notion user for the active profile
notioncli auth use <profile-name>         # switch active profile
notioncli auth logout                     # delete the active profile's stored credential
notioncli auth logout <profile-name>      # delete a specific profile
```

One-shot use without saving a profile:

```bash
NOTION_TOKEN="secret_xxx" notioncli search "roadmap"
```

### search

```bash
notioncli search "roadmap"
notioncli search "eng" --type page --limit 5
notioncli search "" --type data-source --limit 20
```

### page

```bash
# read
notioncli page get "<page-id-or-url>"
notioncli page get "<page-id-or-url>" --include-markdown
notioncli page get "<page-id-or-url>" --include-markdown --include-transcript
notioncli page property "<page-id>" "<property-id>"

# create
notioncli page create --parent-page "<page-id>" --title "New page"
notioncli page create --parent-page "<page-id>" --title "From file" --from-file ./notes.md
notioncli page create --parent-data-source "<ds-id>" --title "New row"
notioncli page create --parent-data-source "<ds-id>" --title-property "Name" --title "New row"
notioncli page create --parent "<id>" --title "Auto-detect parent type"

# append / replace markdown
notioncli page append "<page-id>" --from-file ./append.md
printf "## From CLI\n" | notioncli page append "<page-id>" --stdin
notioncli page replace "<page-id>" --from-file ./replacement.md --allow-deleting-content

# update metadata (raw JSON)
notioncli page update "<page-id>" \
  --body-json '{"cover":{"type":"external","external":{"url":"https://example.com/cover.png"}}}'

# trash / restore
notioncli page trash "<page-id>"
notioncli page restore "<page-id>"
```

`--parent` auto-detects whether the ID is a page or data source (costs one
extra API call). Use `--parent-page` or `--parent-data-source` to skip it.

### block

```bash
notioncli block get "<block-id>"
notioncli block children "<block-id>"
notioncli block children "<block-id>" --page-size 50 --cursor "<cursor>"
notioncli block append "<block-id>" \
  --body-json '{"children":[{"object":"block","type":"paragraph","paragraph":{"rich_text":[{"type":"text","text":{"content":"Hello"}}]}}]}'
notioncli block update "<block-id>" \
  --body-json '{"paragraph":{"rich_text":[{"type":"text","text":{"content":"Updated"}}]}}'
notioncli block delete "<block-id>"
```

### data-source

```bash
notioncli data-source get "<data-source-id>"
notioncli data-source query "<data-source-id>"
notioncli data-source query "<data-source-id>" \
  --filter-json '{"property":"Status","status":{"equals":"In progress"}}' \
  --sort-json '[{"property":"Priority","direction":"descending"}]' \
  --page-size 50
notioncli data-source create --from-file ./data-source.json
notioncli data-source update "<data-source-id>" --from-file ./patch.json
```

### database

```bash
notioncli database get "<database-id>"
notioncli database create --from-file ./database.json
notioncli database update "<database-id>" --from-file ./patch.json
```

### comment

```bash
notioncli comment list "<page-or-block-id>"
notioncli comment get "<comment-id>"
notioncli comment create --from-file ./comment.json
```

### user

```bash
notioncli user me
notioncli user list
notioncli user get "<user-id>"
```

### file-upload

```bash
notioncli file-upload list
notioncli file-upload list --status uploaded
notioncli file-upload get "<file-upload-id>"
notioncli file-upload create --file ./image.png --content-type image/png
```

Currently supports direct single-part uploads only (< 20 MB).

## Pagination

Commands that return lists support `--page-size` and `--cursor`:

```bash
notioncli block children "<id>" --page-size 50
notioncli block children "<id>" --page-size 50 --cursor "<next_cursor>"
```

Applies to: `block children`, `data-source query`, `comment list`, `user list`,
`file-upload list`, `page property`.

## Environment Variables

| Variable | Description |
|---|---|
| `NOTION_TOKEN` | Use this token instead of the saved profile. No credential is stored. |
| `NOTION_API_VERSION` | Override the default Notion API version (`2026-03-11`). |

## Output

Human-readable output is the default. For scripts and automation, use
`--output json`.

```bash
notioncli search "roadmap"                                 # human
notioncli --output json search "roadmap"                   # compact JSON
notioncli --output yaml data-source get "<data-source-id>" # YAML
```

Endpoint commands emit the raw Notion API response. CLI-only commands like
`auth doctor` emit structured CLI-native objects.

Success:

```json
{ "object": "page", "id": "32cee67e-...", "..." : "..." }
```

Error:

```json
{ "object": "error", "status": 400, "code": "validation_error", "message": "..." }
```

## API Coverage

Full endpoint coverage, including partial and intentionally unsupported areas,
is documented in [`API_COVERAGE.md`](./API_COVERAGE.md).

## Live Testing

Two live test scripts run against a real Notion workspace:

- `scripts/live_smoke.sh` — fast auth, output, page, and block checks
- `scripts/live_matrix.sh` — broader pass/fail matrix across all command groups

Requirements: a logged-in profile, `jq`, and a shared parent page (or a
workspace where `search "" --type page --limit 1` can find one).

```bash
scripts/live_smoke.sh
NOTION_TEST_PARENT_ID="<page-id>" scripts/live_matrix.sh
NOTION_TEST_PARENT_ID="<page-id>" NOTION_TEST_KEEP=1 scripts/live_matrix.sh
NOTIONCLI_BIN=./target/release/notioncli scripts/live_matrix.sh
```

The matrix creates disposable pages, comments, uploads, databases, and data
sources under the parent page and trashes the container when done. Set
`NOTION_TEST_KEEP=1` to keep artifacts.

## Benchmarking vs Notion MCP

The repo includes scripts for comparing CLI latency against Notion's hosted MCP
server. The checked-in snapshot is a one-shot tool benchmark (fresh process per
CLI call, fresh session per MCP call).

See [`BENCHMARKS.md`](./BENCHMARKS.md) for results.

Quick start:

```bash
python3 scripts/notion_mcp_login.py
python3 scripts/mcp_list_tools.py --schemas
```

Supported external benchmark:

```bash
python3 scripts/bench_strengthen.py --root-parent-id "<page-id>"
```

Benchmark scripts:

| Script | Purpose |
|---|---|
| `notion_mcp_login.py` | MCP OAuth flow, stores local token |
| `mcp_list_tools.py` | List MCP tools and schemas |
| `bench_case_once.py` | Run one benchmark case |
| `bench_prepare_fixture.py` | Create disposable fixtures |
| `bench_cleanup_fixture.py` | Trash fixture container |
| `bench_strengthen.py` | Repeated suites with fixture isolation |
| `hyperfine_compare.sh` | `hyperfine` runner used by `bench_strengthen.py` |

Case files: `bench_cases.readsuite.json`, `bench_cases.writesuite.json`.

Notes:

- `hyperfine_compare.sh` uses bounded transient retries by default (`BENCH_CASE_RETRIES=2`)
- `NOTION_MCP_ACCESS_TOKEN` overrides the stored MCP token
- Case files support `$ENV_VAR` placeholders
- `bench_strengthen.py` is the supported path for checked-in results

## Notes

- Pins `Notion-Version: 2026-03-11` by default (override with `NOTION_API_VERSION`).
- Profiles and tokens are stored in the platform config directory as local files.
- `NOTION_TOKEN` overrides saved credentials for one-shot use.
- This is a local CLI for the official REST API. No backend, no OAuth dependency.
