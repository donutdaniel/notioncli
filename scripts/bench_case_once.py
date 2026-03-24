#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from notion_mcp_common import (
    DEFAULT_MCP_SERVER_URL,
    NotionMcpClient,
    default_token_store_path,
    load_access_token,
)


RETRYABLE_ERROR_SNIPPETS = (
    "rate limit",
    "retry-after",
    "429",
    "timed out",
    "timeout",
    "temporarily unavailable",
    "service unavailable",
    "bad gateway",
    "gateway timeout",
    "internal server error",
    "connection reset",
    "connection refused",
    "connection aborted",
    "server disconnected",
    "remote end closed connection",
    "empty response",
)


def substitute_env(value: Any) -> Any:
    if isinstance(value, str):
        return os.path.expandvars(value)
    if isinstance(value, list):
        return [substitute_env(item) for item in value]
    if isinstance(value, dict):
        return {key: substitute_env(item) for key, item in value.items()}
    return value


def load_case(path: Path, case_name: str) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(payload, dict):
        cases = payload.get("cases")
    else:
        cases = payload
    if not isinstance(cases, list):
        raise ValueError("case file must be a JSON list or an object with a `cases` list")

    for case in cases:
        if isinstance(case, dict) and case.get("name") == case_name:
            return substitute_env(case)

    raise ValueError(f"benchmark case not found: {case_name}")


def is_retryable_error(message: str) -> bool:
    lowered = message.lower()
    return any(snippet in lowered for snippet in RETRYABLE_ERROR_SNIPPETS)


def run_cli_case(cli_bin: str, argv: list[str], *, retries: int, retry_delay_ms: int) -> int:
    command = [cli_bin, "--output", "json"] + argv
    attempts = retries + 1
    last_completed: subprocess.CompletedProcess[str] | None = None
    last_message = ""

    for attempt in range(1, attempts + 1):
        completed = subprocess.run(command, capture_output=True, text=True, check=False)
        last_completed = completed
        if completed.returncode == 0:
            if completed.stdout:
                print(completed.stdout, end="")
            return 0

        last_message = completed.stderr.strip() or completed.stdout.strip()
        if attempt == attempts or not is_retryable_error(last_message):
            break
        time.sleep(retry_delay_ms / 1000.0)

    if last_completed is not None:
        if last_completed.stdout:
            print(last_completed.stdout, end="")
        if last_completed.stderr:
            print(last_completed.stderr, end="", file=sys.stderr)
        return last_completed.returncode
    if last_message:
        print(last_message, file=sys.stderr)
    return 1


def run_mcp_case(
    *,
    server_url: str,
    token_file: Path,
    tool_name: str,
    arguments: dict[str, Any],
    retries: int,
    retry_delay_ms: int,
) -> int:
    attempts = retries + 1
    last_error = ""

    for attempt in range(1, attempts + 1):
        try:
            access_token, _ = load_access_token(token_file)
            client = NotionMcpClient(server_url=server_url, access_token=access_token)
            client.initialize()
            result = client.call_tool(tool_name, arguments)
            print(json.dumps(result, sort_keys=True))
            return 0
        except Exception as error:  # noqa: BLE001
            last_error = str(error)
            if attempt == attempts or not is_retryable_error(last_error):
                break
            time.sleep(retry_delay_ms / 1000.0)

    if last_error:
        print(last_error, file=sys.stderr)
    return 1


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run a single CLI or MCP benchmark case once."
    )
    parser.add_argument(
        "--system",
        required=True,
        choices=("cli", "mcp"),
        help="Which system to run for this case",
    )
    parser.add_argument(
        "--case-file",
        type=Path,
        default=Path("scripts/bench_cases.readsuite.json"),
        help="JSON case file describing equivalent CLI and MCP operations",
    )
    parser.add_argument(
        "--case-name",
        required=True,
        help="Benchmark case name from the case file",
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
    parser.add_argument(
        "--retries",
        type=int,
        default=int(os.environ.get("BENCH_CASE_RETRIES", "0")),
        help="Retry transient failures this many times before exiting non-zero",
    )
    parser.add_argument(
        "--retry-delay-ms",
        type=int,
        default=int(os.environ.get("BENCH_CASE_RETRY_DELAY_MS", "250")),
        help="Delay between transient retries in milliseconds",
    )
    args = parser.parse_args()

    os.environ.setdefault("BENCH_NONCE", str(time.time_ns()))
    case = load_case(args.case_file, args.case_name)

    if args.system == "cli":
        cli_case = case.get("cli")
        if not isinstance(cli_case, list):
            raise SystemExit(f"case `{args.case_name}` does not define a CLI command")
        exit_code = run_cli_case(
            args.cli_bin,
            cli_case,
            retries=max(args.retries, 0),
            retry_delay_ms=max(args.retry_delay_ms, 0),
        )
        raise SystemExit(exit_code)

    mcp_case = case.get("mcp")
    if not isinstance(mcp_case, dict):
        raise SystemExit(f"case `{args.case_name}` does not define an MCP tool call")
    tool_name = mcp_case.get("tool")
    arguments = mcp_case.get("arguments", {})
    if not isinstance(tool_name, str):
        raise SystemExit(f"case `{args.case_name}` has an invalid MCP tool name")
    if not isinstance(arguments, dict):
        raise SystemExit(f"case `{args.case_name}` has invalid MCP arguments")

    exit_code = run_mcp_case(
        server_url=args.mcp_server_url,
        token_file=args.mcp_token_file,
        tool_name=tool_name,
        arguments=arguments,
        retries=max(args.retries, 0),
        retry_delay_ms=max(args.retry_delay_ms, 0),
    )
    raise SystemExit(exit_code)


if __name__ == "__main__":
    main()
