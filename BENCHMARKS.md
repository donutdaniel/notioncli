# Benchmarks

This file captures the latest strengthened benchmark snapshot for `notioncli`
versus Notion's hosted MCP server.

Important: this is a one-shot external tool benchmark. Each measurement starts
from a fresh local invocation:

- CLI: a fresh `notioncli` process per run
- MCP: a fresh MCP session per run

It does not measure a reused-session MCP client, and it does not measure a
persistent long-lived CLI process. Read the results as "one-shot local tool
latency" rather than a universal claim about every possible CLI vs MCP
integration model.

## Method

- Runner: `hyperfine 1.20.0`
- CLI binary: `./target/release/notioncli`
- MCP server: `https://mcp.notion.com/mcp`
- Orchestrator: `scripts/bench_strengthen.py`
- Reported metric: median of session medians from the exported `hyperfine` JSON
  files in `tmp/bench-strengthened/`
- Comparison mode:
  - CLI: fresh process per run
  - MCP: fresh session per run

### Read Suite

- Case file: `scripts/bench_cases.readsuite.json`
- Fixture strategy: one fixed disposable fixture set per session
- Sessions: `2`
- Runs per case: `20`
- Warmup per case: `3`

### Write Suite

- Case file: `scripts/bench_cases.writesuite.json`
- Fixture strategy: one isolated disposable fixture set per case, per session
- Sessions: `2`
- Runs per case: `10`
- Warmup per case: `2`

This is a stronger snapshot than the earlier mixed 5-run comparable suite
because it separates reads from writes, isolates mutating cases, and repeats the
full suite across multiple sessions.

## Read Suite

| Case | CLI median of medians (ms) | MCP median of medians (ms) | Winner | Ratio |
|---|---:|---:|---|---:|
| `current-user` | 285.0 | 1091.2 | CLI | CLI 3.83x faster |
| `fetch-data-source` | 363.7 | 1626.0 | CLI | CLI 4.47x faster |
| `fetch-database` | 410.4 | 1542.2 | CLI | CLI 3.76x faster |
| `fetch-page` | 405.7 | 1429.8 | CLI | CLI 3.52x faster |
| `list-comments` | 455.1 | 1186.1 | CLI | CLI 2.61x faster |
| `list-users-page1` | 285.6 | 1113.8 | CLI | CLI 3.90x faster |
| `search-page` | 446.2 | 1694.1 | CLI | CLI 3.80x faster |

## Write Suite

| Case | CLI median of medians (ms) | MCP median of medians (ms) | Winner | Ratio |
|---|---:|---:|---|---:|
| `create-comment` | 921.8 | 1187.7 | CLI | CLI 1.29x faster |
| `create-database` | 861.1 | 1423.7 | CLI | CLI 1.65x faster |
| `create-page-under-page` | 851.1 | 1396.4 | CLI | CLI 1.64x faster |
| `create-row-under-data-source` | 1222.7 | 1261.4 | CLI | CLI 1.03x faster |
| `update-data-source-title` | 903.0 | 1154.9 | CLI | CLI 1.28x faster |
| `update-page-title` | 856.9 | 1166.9 | CLI | CLI 1.36x faster |

## Notes

- CLI won `13/13` cases in this strengthened snapshot.
- The read-path advantage is large and stable in this run, generally between
  `2.61x` and `4.47x`.
- The write-path advantage is smaller, generally between `1.03x` and `1.65x`.
  `create-row-under-data-source` is effectively a near-tie.
- `create-page-under-page` flipped in favor of the CLI after adding explicit
  `--parent-page` and `--parent-data-source` flags and removing the older
  parent autodetection overhead from the hot path.
- This remains a directional internal benchmark, not a publishable external
  claim. The comparison is between:
  - a local Rust CLI calling the public Notion REST API
  - Notion's first-party hosted MCP service
- The headline result should be read narrowly: in this repo's current setup,
  one-shot CLI invocations were faster than one-shot fresh-session MCP calls.
- A reused-session MCP client may perform differently, and a future persistent
  CLI mode would also change the comparison.
- The older mixed comparable-suite snapshot is superseded by this one.
