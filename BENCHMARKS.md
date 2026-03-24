# Benchmarks

This file captures the latest benchmark snapshot for `notioncli` versus
Notion's hosted MCP server.

Important: this is a one-shot end-to-end latency benchmark from one machine.
Each measurement starts from a fresh local invocation:

- CLI: a fresh `notioncli` process per run
- MCP: a fresh MCP session per run

The numbers include more than just "the network" or "the protocol". They also
include local startup/config loading, request construction, response parsing,
network latency, and remote Notion or MCP service time.

It does not measure a reused-session MCP client, and it does not measure a
persistent long-lived CLI process. This is not a pure protocol benchmark. Read
the results as "one-shot local tool latency from this machine" rather than a
universal claim about every CLI vs MCP integration model.

## Method

- Snapshot date: `2026-03-24 11:43:27 EDT`
- Code state: local uncommitted worktree during the benchmark run
- Machine: `Apple M4 Max`
- OS: `macOS 26.2 (25C56)`
- Runner: `hyperfine 1.20.0`
- CLI binary: `./target/release/notioncli`
- MCP server: `https://mcp.notion.com/mcp`
- Orchestrator: `scripts/bench_strengthen.py`
- Raw artifacts: `benchmarks/publish-20260324/`
- Reported metric: median of session medians from the exported `hyperfine` JSON
  files in `benchmarks/publish-20260324/`
- Backend dependency: live Notion REST API and hosted MCP service
- Transient retry policy: `bench_case_once.py --retries 2 --retry-delay-ms 250`
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

This snapshot separates reads from writes, isolates mutating cases, and repeats
the suite across multiple sessions.

## Read Suite

| Case | CLI median of medians (ms) | MCP median of medians (ms) | Ratio |
|---|---:|---:|---:|
| `current-user` | 332.5 | 822.6 | CLI 2.47x faster |
| `fetch-data-source` | 399.7 | 1981.2 | CLI 4.96x faster |
| `fetch-database` | 396.1 | 1984.0 | CLI 5.01x faster |
| `fetch-page` | 480.2 | 1438.5 | CLI 3.00x faster |
| `list-comments` | 431.5 | 1414.6 | CLI 3.28x faster |
| `list-users-page1` | 320.1 | 1119.1 | CLI 3.50x faster |
| `search-page` | 499.5 | 1849.0 | CLI 3.70x faster |

## Write Suite

| Case | CLI median of medians (ms) | MCP median of medians (ms) | Ratio |
|---|---:|---:|---:|
| `create-comment` | 770.6 | 1315.7 | CLI 1.71x faster |
| `create-database` | 1588.0 | 1389.0 | MCP 1.14x faster |
| `create-page-under-page` | 673.5 | 1414.9 | CLI 2.10x faster |
| `create-row-under-data-source` | 1284.2 | 1639.8 | CLI 1.28x faster |
| `update-data-source-title` | 1122.5 | 2190.6 | CLI 1.95x faster |
| `update-page-title` | 936.0 | 1220.5 | CLI 1.30x faster |

## Notes

- The CLI was faster in `12/13` cases in this snapshot.
- In this snapshot, the read cases land between `2.47x` and `5.01x` in favor of
  the CLI.
- In this snapshot, the write cases range from `MCP 1.14x faster` on
  `create-database` to `CLI 2.10x faster` on `create-page-under-page`.
- `create-database` favored MCP in this run, so the write suite is not a clean
  sweep for the CLI.
- Several write cases still show high session-to-session variance, especially
  `create-comment`, `create-database`, `update-page-title`, and
  `update-data-source-title`. Small write-path gaps should be treated
  cautiously.
- This is still a directional benchmark, not a protocol proof.
- The comparison is between a local Rust CLI calling the public Notion REST API
  and Notion's hosted MCP service.
- The headline result should be read narrowly: in this repo's current setup,
  one-shot CLI invocations were usually faster than one-shot fresh-session MCP
  calls.
- A reused-session MCP client may perform differently, and a future persistent
  CLI mode would also change the comparison.
- A different machine, network path, time of day, or Notion-side load can move
  these numbers around.
