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


def run_cli_case(cli_bin: str, argv: list[str]) -> int:
    completed = subprocess.run([cli_bin, "--output", "json"] + argv, check=False)
    return completed.returncode


def run_mcp_case(
    *,
    server_url: str,
    token_file: Path,
    tool_name: str,
    arguments: dict[str, Any],
) -> int:
    access_token, _ = load_access_token(token_file)
    client = NotionMcpClient(server_url=server_url, access_token=access_token)
    client.initialize()
    result = client.call_tool(tool_name, arguments)
    print(json.dumps(result, sort_keys=True))
    return 0


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
        default=Path("scripts/bench_cases.readonly.json"),
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
    args = parser.parse_args()

    os.environ.setdefault("BENCH_NONCE", str(time.time_ns()))
    case = load_case(args.case_file, args.case_name)

    if args.system == "cli":
        cli_case = case.get("cli")
        if not isinstance(cli_case, list):
            raise SystemExit(f"case `{args.case_name}` does not define a CLI command")
        exit_code = run_cli_case(args.cli_bin, cli_case)
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
    )
    raise SystemExit(exit_code)


if __name__ == "__main__":
    main()
