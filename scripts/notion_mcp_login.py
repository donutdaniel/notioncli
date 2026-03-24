#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

from notion_mcp_common import (
    DEFAULT_MCP_SERVER_URL,
    default_pending_oauth_path,
    default_token_store_path,
    start_oauth_login,
    perform_oauth_login,
)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Log in to Notion's hosted MCP server with OAuth PKCE and store a local access token."
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
        help="Path where the OAuth token bundle should be stored",
    )
    parser.add_argument(
        "--no-browser",
        action="store_true",
        help="Print the authorization URL instead of opening a browser automatically",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=int,
        default=300,
        help="Maximum time to wait for the OAuth callback",
    )
    parser.add_argument(
        "--callback-url",
        help="Optional full redirect URL to use instead of waiting for a localhost callback",
    )
    parser.add_argument(
        "--pending-file",
        type=Path,
        default=default_pending_oauth_path(),
        help="Path where the pending OAuth login context should be stored",
    )
    parser.add_argument(
        "--start-only",
        action="store_true",
        help="Create a pending OAuth login context, print the authorization URL, and exit",
    )
    args = parser.parse_args()

    if args.start_only:
        start_oauth_login(
            server_url=args.server_url,
            pending_path=args.pending_file,
            open_browser=not args.no_browser,
        )
        return

    path = perform_oauth_login(
        server_url=args.server_url,
        token_path=args.token_file,
        pending_path=args.pending_file,
        open_browser=not args.no_browser,
        timeout_seconds=args.timeout_seconds,
        manual_callback_url=args.callback_url,
    )
    print(f"Saved Notion MCP OAuth tokens to {path}")


if __name__ == "__main__":
    main()
