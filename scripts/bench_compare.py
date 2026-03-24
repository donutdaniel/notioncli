#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import time
from pathlib import Path
from typing import Any

from notion_mcp_common import (
    DEFAULT_MCP_SERVER_URL,
    NotionMcpClient,
    default_token_store_path,
    load_access_token,
)


def substitute_env(value: Any) -> Any:
    if isinstance(value, str):
        return os.path.expandvars(value)
    if isinstance(value, list):
        return [substitute_env(item) for item in value]
    if isinstance(value, dict):
        return {key: substitute_env(item) for key, item in value.items()}
    return value


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    index = round((len(values) - 1) * fraction)
    return sorted(values)[index]


def summarize_timings(timings_ms: list[float]) -> dict[str, float]:
    sorted_values = sorted(timings_ms)
    return {
        "min_ms": sorted_values[0],
        "median_ms": statistics.median(sorted_values),
        "mean_ms": statistics.fmean(sorted_values),
        "p95_ms": percentile(sorted_values, 0.95),
        "max_ms": sorted_values[-1],
    }


def benchmark_nonce(phase: str, run_index: int) -> str:
    return f"{time.time_ns()}-{phase}-{run_index}"


def substitute_env_with_nonce(value: Any, nonce: str) -> Any:
    previous = os.environ.get("BENCH_NONCE")
    os.environ["BENCH_NONCE"] = nonce
    try:
        return substitute_env(value)
    finally:
        if previous is None:
            os.environ.pop("BENCH_NONCE", None)
        else:
            os.environ["BENCH_NONCE"] = previous


def summarize_case_rows(
    *,
    case_name: str,
    system: str,
    mode: str,
    rows: list[dict[str, Any]],
    timings: list[float],
) -> dict[str, Any]:
    summary: dict[str, Any] = {
        "case": case_name,
        "system": system,
        "mode": mode,
        "success_rate": (sum(1 for row in rows if row["ok"]) / len(rows)) if rows else 0.0,
    }
    if timings:
        summary.update(summarize_timings(timings))
    else:
        summary.update(
            {
                "min_ms": None,
                "median_ms": None,
                "mean_ms": None,
                "p95_ms": None,
                "max_ms": None,
            }
        )
    return summary


def benchmark_cli_case(
    *,
    cli_bin: str,
    argv_template: list[str],
    runs: int,
    warmup: int,
) -> tuple[list[dict[str, Any]], list[float]]:
    rows: list[dict[str, Any]] = []
    timings: list[float] = []

    for phase in ("warmup", "measure"):
        count = warmup if phase == "warmup" else runs
        for run_index in range(1, count + 1):
            command = [cli_bin, "--output", "json"] + substitute_env_with_nonce(
                argv_template,
                benchmark_nonce(phase, run_index),
            )
            start = time.perf_counter()
            completed = subprocess.run(
                command,
                capture_output=True,
                text=True,
                check=False,
            )
            elapsed_ms = (time.perf_counter() - start) * 1000.0
            if phase == "measure":
                row = {
                    "system": "cli",
                    "mode": "process",
                    "run": run_index,
                    "ok": completed.returncode == 0,
                    "ms": elapsed_ms,
                    "exit_code": completed.returncode,
                }
                if completed.returncode != 0:
                    row["error"] = completed.stderr.strip() or completed.stdout.strip()
                rows.append(row)
                if completed.returncode == 0:
                    timings.append(elapsed_ms)

    return rows, timings


def benchmark_mcp_case_reused(
    *,
    client: NotionMcpClient,
    tool_name_template: str,
    arguments_template: dict[str, Any],
    runs: int,
    warmup: int,
) -> tuple[list[dict[str, Any]], list[float]]:
    rows: list[dict[str, Any]] = []
    timings: list[float] = []

    for phase in ("warmup", "measure"):
        count = warmup if phase == "warmup" else runs
        for run_index in range(1, count + 1):
            error_message = None
            nonce = benchmark_nonce(phase, run_index)
            tool_name = substitute_env_with_nonce(tool_name_template, nonce)
            arguments = substitute_env_with_nonce(arguments_template, nonce)
            start = time.perf_counter()
            try:
                client.call_tool(tool_name, arguments)
                ok = True
            except Exception as error:  # noqa: BLE001
                ok = False
                error_message = str(error)
            elapsed_ms = (time.perf_counter() - start) * 1000.0

            if phase == "measure":
                row = {
                    "system": "mcp",
                    "mode": "reused_session",
                    "run": run_index,
                    "ok": ok,
                    "ms": elapsed_ms,
                }
                if error_message:
                    row["error"] = error_message
                rows.append(row)
                if ok:
                    timings.append(elapsed_ms)

    return rows, timings


def benchmark_mcp_case_fresh(
    *,
    server_url: str,
    access_token: str,
    tool_name_template: str,
    arguments_template: dict[str, Any],
    runs: int,
    warmup: int,
) -> tuple[list[dict[str, Any]], list[float]]:
    rows: list[dict[str, Any]] = []
    timings: list[float] = []

    for phase in ("warmup", "measure"):
        count = warmup if phase == "warmup" else runs
        for run_index in range(1, count + 1):
            error_message = None
            nonce = benchmark_nonce(phase, run_index)
            tool_name = substitute_env_with_nonce(tool_name_template, nonce)
            arguments = substitute_env_with_nonce(arguments_template, nonce)
            start = time.perf_counter()
            try:
                client = NotionMcpClient(server_url=server_url, access_token=access_token)
                client.initialize()
                client.call_tool(tool_name, arguments)
                ok = True
            except Exception as error:  # noqa: BLE001
                ok = False
                error_message = str(error)
            elapsed_ms = (time.perf_counter() - start) * 1000.0

            if phase == "measure":
                row = {
                    "system": "mcp",
                    "mode": "fresh_session",
                    "run": run_index,
                    "ok": ok,
                    "ms": elapsed_ms,
                }
                if error_message:
                    row["error"] = error_message
                rows.append(row)
                if ok:
                    timings.append(elapsed_ms)

    return rows, timings


def load_case_file(path: Path) -> list[dict[str, Any]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(payload, dict):
        cases = payload.get("cases")
    else:
        cases = payload
    if not isinstance(cases, list):
        raise ValueError("case file must be a JSON list or an object with a `cases` list")
    return cases


def print_summary(summary_rows: list[dict[str, Any]]) -> None:
    def format_metric(value: Any) -> str:
        if value is None:
            return "n/a"
        return f"{value:.1f}"

    print("SUMMARY")
    for row in summary_rows:
        status = "PASS" if row["success_rate"] == 1.0 else "FAIL"
        print(
            f"{status} case={row['case']} system={row['system']} mode={row['mode']} "
            f"success_rate={row['success_rate']:.2f} "
            f"median_ms={format_metric(row['median_ms'])} p95_ms={format_metric(row['p95_ms'])}"
        )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Benchmark notioncli against Notion's hosted MCP tools."
    )
    parser.add_argument(
        "--case-file",
        type=Path,
        default=Path("scripts/bench_cases.sample.json"),
        help="JSON case file describing equivalent CLI and MCP operations",
    )
    parser.add_argument(
        "--cli-bin",
        default="./target/release/notioncli",
        help="Path to the notioncli binary to benchmark",
    )
    parser.add_argument(
        "--mcp-server-url",
        default=DEFAULT_MCP_SERVER_URL,
        help=f"Hosted MCP server URL (default: {DEFAULT_MCP_SERVER_URL})",
    )
    parser.add_argument(
        "--mcp-token-file",
        type=Path,
        default=default_token_store_path(),
        help="Path to the stored MCP OAuth token bundle",
    )
    parser.add_argument("--runs", type=int, default=20, help="Measured runs per case")
    parser.add_argument("--warmup", type=int, default=3, help="Warmup runs per case")
    parser.add_argument(
        "--systems",
        default="cli,mcp",
        help="Comma-separated systems to benchmark: cli,mcp",
    )
    parser.add_argument(
        "--mcp-modes",
        default="reused_session,fresh_session",
        help="Comma-separated MCP measurement modes: reused_session,fresh_session",
    )
    parser.add_argument(
        "--jsonl-out",
        type=Path,
        help="Optional JSONL file to receive per-run benchmark rows",
    )
    parser.add_argument(
        "--summary-out",
        type=Path,
        help="Optional JSON file to receive summarized benchmark rows",
    )
    args = parser.parse_args()

    systems = {item.strip() for item in args.systems.split(",") if item.strip()}
    mcp_modes = {item.strip() for item in args.mcp_modes.split(",") if item.strip()}
    cases = load_case_file(args.case_file)

    per_run_rows: list[dict[str, Any]] = []
    summary_rows: list[dict[str, Any]] = []

    access_token = None
    shared_mcp_client = None
    if "mcp" in systems:
        access_token, _ = load_access_token(args.mcp_token_file)
        if "reused_session" in mcp_modes:
            shared_mcp_client = NotionMcpClient(
                server_url=args.mcp_server_url,
                access_token=access_token,
            )
            shared_mcp_client.initialize()

    for case in cases:
        case_name = case.get("name")
        if not case_name:
            raise ValueError("every benchmark case must have a `name`")

        if "cli" in systems and case.get("cli"):
            cli_rows, cli_timings = benchmark_cli_case(
                cli_bin=args.cli_bin,
                argv_template=case["cli"],
                runs=args.runs,
                warmup=args.warmup,
            )
            for row in cli_rows:
                row["case"] = case_name
            per_run_rows.extend(cli_rows)
            summary_rows.append(
                summarize_case_rows(
                    case_name=case_name,
                    system="cli",
                    mode="process",
                    rows=cli_rows,
                    timings=cli_timings,
                )
            )

        if "mcp" in systems and case.get("mcp"):
            tool_name_template = case["mcp"]["tool"]
            arguments_template = case["mcp"].get("arguments", {})

            if "reused_session" in mcp_modes and shared_mcp_client is not None:
                mcp_rows, mcp_timings = benchmark_mcp_case_reused(
                    client=shared_mcp_client,
                    tool_name_template=tool_name_template,
                    arguments_template=arguments_template,
                    runs=args.runs,
                    warmup=args.warmup,
                )
                for row in mcp_rows:
                    row["case"] = case_name
                per_run_rows.extend(mcp_rows)
                summary_rows.append(
                    summarize_case_rows(
                        case_name=case_name,
                        system="mcp",
                        mode="reused_session",
                        rows=mcp_rows,
                        timings=mcp_timings,
                    )
                )

            if "fresh_session" in mcp_modes and access_token is not None:
                fresh_rows, fresh_timings = benchmark_mcp_case_fresh(
                    server_url=args.mcp_server_url,
                    access_token=access_token,
                    tool_name_template=tool_name_template,
                    arguments_template=arguments_template,
                    runs=args.runs,
                    warmup=args.warmup,
                )
                for row in fresh_rows:
                    row["case"] = case_name
                per_run_rows.extend(fresh_rows)
                summary_rows.append(
                    summarize_case_rows(
                        case_name=case_name,
                        system="mcp",
                        mode="fresh_session",
                        rows=fresh_rows,
                        timings=fresh_timings,
                    )
                )

    if args.jsonl_out:
        args.jsonl_out.write_text(
            "".join(json.dumps(row, sort_keys=True) + "\n" for row in per_run_rows),
            encoding="utf-8",
        )

    if args.summary_out:
        args.summary_out.write_text(
            json.dumps(summary_rows, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    print_summary(summary_rows)


if __name__ == "__main__":
    main()
