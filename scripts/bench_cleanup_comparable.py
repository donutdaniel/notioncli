#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import subprocess
import sys


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Trash the disposable container page used for comparable benchmarks."
    )
    parser.add_argument(
        "--cli-bin",
        default="./target/release/notioncli",
        help="Path to the notioncli binary",
    )
    parser.add_argument(
        "--container-page-id",
        default=os.environ.get("CONTAINER_PAGE_ID", ""),
        help="Container page id to trash",
    )
    args = parser.parse_args()

    if not args.container_page_id:
        raise SystemExit("missing container page id; set CONTAINER_PAGE_ID or pass --container-page-id")

    completed = subprocess.run(
        [args.cli_bin, "--output", "json", "page", "trash", args.container_page_id],
        check=False,
    )
    raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
