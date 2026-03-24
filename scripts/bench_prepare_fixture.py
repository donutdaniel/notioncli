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


def run_cli(cli_bin: str, *args: str) -> dict[str, Any]:
    completed = subprocess.run(
        [cli_bin, "--output", "json", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        message = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"command failed ({completed.returncode}): {cli_bin} {' '.join(args)}\n{message}")
    return json.loads(completed.stdout)


def rich_text(text: str) -> list[dict[str, Any]]:
    return [{"type": "text", "text": {"content": text}}]


def discover_root_parent(cli_bin: str) -> str:
    search = run_cli(cli_bin, "search", "", "--type", "page", "--limit", "1")
    results = search.get("results", [])
    if not results:
        raise RuntimeError(
            "could not auto-discover a shared parent page; set NOTION_TEST_PARENT_ID"
        )
    first = results[0]
    page_id = first.get("id")
    if not isinstance(page_id, str) or not page_id:
        raise RuntimeError("search result did not include a page id")
    return page_id


def print_shell_exports(values: dict[str, str]) -> None:
    for key, value in values.items():
        print(f"export {key}={json.dumps(value)}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Create a disposable fixture set for the benchmark suite."
    )
    parser.add_argument(
        "--cli-bin",
        default="./target/release/notioncli",
        help="Path to the notioncli binary",
    )
    parser.add_argument(
        "--root-parent-id",
        default=os.environ.get("NOTION_TEST_PARENT_ID", ""),
        help="Shared parent page for fixture creation; falls back to search discovery",
    )
    parser.add_argument(
        "--prefix",
        default=f"bench-{int(time.time())}",
        help="Prefix used in fixture titles and search queries",
    )
    parser.add_argument(
        "--format",
        choices=("shell", "json"),
        default="shell",
        help="Output format",
    )
    args = parser.parse_args()

    root_parent_id = args.root_parent_id or discover_root_parent(args.cli_bin)

    container = run_cli(
        args.cli_bin,
        "page",
        "create",
        "--parent-page",
        root_parent_id,
        "--title",
        f"{args.prefix}-container",
    )
    container_page_id = container["id"]

    content = run_cli(
        args.cli_bin,
        "page",
        "create",
        "--parent-page",
        container_page_id,
        "--title",
        f"{args.prefix}-content-page",
    )
    content_page_id = content["id"]

    run_cli(
        args.cli_bin,
        "comment",
        "create",
        "--body-json",
        json.dumps(
            {
                "parent": {"page_id": content_page_id},
                "rich_text": rich_text(f"{args.prefix} initial comment"),
            }
        ),
    )

    database = run_cli(
        args.cli_bin,
        "database",
        "create",
        "--body-json",
        json.dumps(
            {
                "parent": {"type": "page_id", "page_id": container_page_id},
                "title": rich_text(f"{args.prefix} database"),
                "initial_data_source": {
                    "title": rich_text("Primary"),
                    "properties": {
                        "Name": {"title": {}},
                        "Notes": {"rich_text": {}},
                    },
                },
            }
        ),
    )
    database_id = database["id"]
    data_sources = database.get("data_sources", [])
    if data_sources:
        data_source_id = data_sources[0]["id"]
    else:
        data_source_id = database["initial_data_source"]["id"]

    values = {
        "ROOT_PARENT_ID": root_parent_id,
        "CONTAINER_PAGE_ID": container_page_id,
        "CONTENT_PAGE_ID": content_page_id,
        "DATABASE_ID": database_id,
        "DATA_SOURCE_ID": data_source_id,
        "SEARCH_QUERY": f"{args.prefix}-content-page",
        "BENCH_PREFIX": args.prefix,
    }

    if args.format == "json":
        print(json.dumps(values, indent=2, sort_keys=True))
    else:
        print_shell_exports(values)


if __name__ == "__main__":
    main()
