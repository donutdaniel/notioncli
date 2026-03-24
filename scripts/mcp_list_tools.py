#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from pathlib import Path

from notion_mcp_common import (
    DEFAULT_MCP_SERVER_URL,
    NotionMcpClient,
    default_token_store_path,
    load_access_token,
)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="List tools exposed by Notion's hosted MCP server."
    )
    parser.add_argument(
        "--server-url",
        default=DEFAULT_MCP_SERVER_URL,
        help=f"Hosted MCP server URL (default: {DEFAULT_MCP_SERVER_URL})",
    )
    parser.add_argument(
        "--token-file",
        type=Path,
        default=default_token_store_path(),
        help="Path to the stored MCP OAuth token bundle",
    )
    parser.add_argument(
        "--schemas",
        action="store_true",
        help="Include the full input schema for each tool",
    )
    args = parser.parse_args()

    access_token, _ = load_access_token(args.token_file)
    client = NotionMcpClient(server_url=args.server_url, access_token=access_token)
    client.initialize()
    tools = client.list_tools()

    if not args.schemas:
        tools = [
            {
                "name": tool.get("name"),
                "description": tool.get("description"),
            }
            for tool in tools
        ]

    print(
        json.dumps(
            {
                "server_url": args.server_url,
                "tool_count": len(tools),
                "tools": tools,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
