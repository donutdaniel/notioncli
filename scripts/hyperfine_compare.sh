#!/usr/bin/env bash
set -euo pipefail

if ! command -v hyperfine >/dev/null 2>&1; then
  echo "hyperfine is required. Install it first, for example: brew install hyperfine" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case_file="${CASE_FILE:-$repo_root/scripts/bench_cases.readsuite.json}"
cli_bin="${CLI_BIN:-$repo_root/target/release/notioncli}"
out_dir="${OUT_DIR:-$repo_root/tmp/hyperfine}"
runs="${RUNS:-20}"
warmup="${WARMUP:-3}"
extra_args="${HYPERFINE_ARGS:-}"
retry_count="${BENCH_CASE_RETRIES:-2}"
retry_delay_ms="${BENCH_CASE_RETRY_DELAY_MS:-250}"

mkdir -p "$out_dir"

if [[ $# -eq 0 ]]; then
  cases=()
  while IFS= read -r line; do
    [[ -n "$line" ]] && cases+=("$line")
  done < <(
    python3 - "$case_file" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
payload = json.loads(path.read_text(encoding="utf-8"))
cases = payload.get("cases") if isinstance(payload, dict) else payload
for case in cases:
    if isinstance(case, dict) and isinstance(case.get("name"), str):
        print(case["name"])
PY
  )
else
  cases=("$@")
fi

for case_name in "${cases[@]}"; do
  echo "== $case_name =="
  printf -v cli_cmd 'python3 %q --system cli --case-file %q --case-name %q --cli-bin %q --retries %q --retry-delay-ms %q >/dev/null' \
    "$repo_root/scripts/bench_case_once.py" "$case_file" "$case_name" "$cli_bin" "$retry_count" "$retry_delay_ms"
  printf -v mcp_cmd 'python3 %q --system mcp --case-file %q --case-name %q --retries %q --retry-delay-ms %q >/dev/null' \
    "$repo_root/scripts/bench_case_once.py" "$case_file" "$case_name" "$retry_count" "$retry_delay_ms"

  # shellcheck disable=SC2086
  hyperfine \
    --warmup "$warmup" \
    --runs "$runs" \
    --export-json "$out_dir/$case_name.json" \
    $extra_args \
    -n cli "$cli_cmd" \
    -n mcp "$mcp_cmd"
done
