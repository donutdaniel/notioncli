#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export NOTION_TEST_SCOPE=smoke
exec "$SCRIPT_DIR/live_matrix.sh" "$@"
