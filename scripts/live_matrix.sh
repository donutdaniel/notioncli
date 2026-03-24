#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BIN="${NOTIONCLI_BIN:-${NOTION_CLI_BIN:-$REPO_ROOT/target/debug/notioncli}}"
TEST_PARENT_ID="${NOTION_TEST_PARENT_ID:-}"
KEEP_ARTIFACTS="${NOTION_TEST_KEEP:-0}"
SCOPE="${NOTION_TEST_SCOPE:-matrix}"
PROFILE="${NOTIONCLI_PROFILE:-${NOTION_CLI_PROFILE:-}}"
ACTIVE_PROFILE=""
STAMP="$(date +%s)"
PREFIX="cli-live-${STAMP}"
TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/notioncli-live.XXXXXX")"
REPORT_TSV="$TMPDIR/report.tsv"
touch "$REPORT_TSV"

CLI_ARGS=("$BIN" "--output" "json")
if [[ -n "$PROFILE" ]]; then
  CLI_ARGS+=(--profile "$PROFILE")
fi

LAST_STATUS=0
LAST_OUTPUT=""

ROOT_PARENT_ID=""
CONTAINER_PAGE_ID=""
CONTENT_PAGE_ID=""
TRASH_PAGE_ID=""
BLOCK_ID=""
COMMENT_ID=""
COMMENT_ID_2=""
DATABASE_ID=""
DATA_SOURCE_ID=""
ROW_ID_1=""
ROW_ID_2=""
SECONDARY_DATA_SOURCE_ID=""
FILE_UPLOAD_ID=""

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 2
  fi
}

ensure_binary() {
  if [[ -f "$REPO_ROOT/Cargo.toml" ]] && command -v cargo >/dev/null 2>&1 \
    && [[ "$BIN" == "$REPO_ROOT"/target/* ]]; then
    (cd "$REPO_ROOT" && cargo build >/dev/null)
  fi

  if [[ ! -x "$BIN" ]]; then
    echo "notioncli binary not found at $BIN" >&2
    exit 2
  fi
}

cli() {
  "${CLI_ARGS[@]}" "$@"
}

record() {
  local status="$1"
  local name="$2"
  local detail="$3"
  printf '%s\t%s\t%s\n' "$status" "$name" "$detail" >> "$REPORT_TSV"
}

compact_output() {
  printf '%s' "$1" | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g'
}

run_success_case() {
  local name="$1"
  shift
  set +e
  LAST_OUTPUT="$("$@" 2>&1)"
  LAST_STATUS=$?
  set -e

  if [[ $LAST_STATUS -eq 0 ]]; then
    record PASS "$name" "ok"
  else
    record FAIL "$name" "$(compact_output "$LAST_OUTPUT")"
  fi
}

run_error_case() {
  local name="$1"
  local expected_code="$2"
  shift 2
  set +e
  LAST_OUTPUT="$("$@" 2>&1)"
  LAST_STATUS=$?
  set -e

  if [[ $LAST_STATUS -eq 0 ]]; then
    record FAIL "$name" "expected error code $expected_code but command succeeded"
    return
  fi

  local actual_code
  actual_code="$(printf '%s' "$LAST_OUTPUT" | jq -r '.code // empty' 2>/dev/null || true)"
  if [[ "$actual_code" == "$expected_code" ]]; then
    record PASS "$name" "error:$expected_code"
  else
    record FAIL "$name" "expected error:$expected_code got '${actual_code:-unknown}' $(compact_output "$LAST_OUTPUT")"
  fi
}

run_any_error_case() {
  local name="$1"
  shift
  set +e
  LAST_OUTPUT="$("$@" 2>&1)"
  LAST_STATUS=$?
  set -e

  if [[ $LAST_STATUS -eq 0 ]]; then
    record FAIL "$name" "expected command to fail but it succeeded"
    return
  fi

  local actual_code
  actual_code="$(printf '%s' "$LAST_OUTPUT" | jq -r '.code // empty' 2>/dev/null || true)"
  record PASS "$name" "error:${actual_code:-unknown}"
}

run_text_case() {
  local name="$1"
  local expected_pattern="$2"
  shift 2
  set +e
  LAST_OUTPUT="$("$@" 2>&1)"
  LAST_STATUS=$?
  set -e

  if [[ $LAST_STATUS -eq 0 ]] && printf '%s' "$LAST_OUTPUT" | grep -Eq "$expected_pattern"; then
    record PASS "$name" "matched:$expected_pattern"
  else
    record FAIL "$name" "$(compact_output "$LAST_OUTPUT")"
  fi
}

run_text_error_case() {
  local name="$1"
  local expected_pattern="$2"
  shift 2
  set +e
  LAST_OUTPUT="$("$@" 2>&1)"
  LAST_STATUS=$?
  set -e

  if [[ $LAST_STATUS -ne 0 ]] && printf '%s' "$LAST_OUTPUT" | grep -Eq "$expected_pattern"; then
    record PASS "$name" "matched:$expected_pattern"
  else
    record FAIL "$name" "$(compact_output "$LAST_OUTPUT")"
  fi
}

skip_case() {
  record SKIP "$1" "$2"
}

json_value() {
  local filter="$1"
  printf '%s' "$LAST_OUTPUT" | jq -r "$filter"
}

write_fixture_files() {
  printf '# content page\ninitial body\n' > "$TMPDIR/content.md"
  printf 'appended line\n' > "$TMPDIR/append.md"
  printf '# replaced\nfinal body\n' > "$TMPDIR/replace.md"
  printf 'upload smoke %s\n' "$STAMP" > "$TMPDIR/upload.txt"
  dd if=/dev/zero of="$TMPDIR/too-large.bin" bs=1048576 count=6 status=none
}

discover_parent() {
  if [[ -n "$TEST_PARENT_ID" ]]; then
    ROOT_PARENT_ID="$TEST_PARENT_ID"
    record PASS "discover_parent" "env:$ROOT_PARENT_ID"
    return
  fi

  run_success_case "discover_parent_search" cli search "" --type page --limit 10
  if [[ $LAST_STATUS -ne 0 ]]; then
    record FAIL "discover_parent" "search failed"
    return
  fi

  ROOT_PARENT_ID="$(json_value '.results[0].id // empty')"
  if [[ -n "$ROOT_PARENT_ID" ]]; then
    record PASS "discover_parent" "$ROOT_PARENT_ID"
  else
    record FAIL "discover_parent" "no shared page found; set NOTION_TEST_PARENT_ID"
  fi
}

run_core_auth_and_output_cases() {
  run_success_case "auth_doctor" cli auth doctor
  run_success_case "auth_whoami" cli auth whoami
  run_success_case "auth_list" cli auth list
  if [[ $LAST_STATUS -eq 0 ]]; then
    ACTIVE_PROFILE="$(json_value '.profiles[] | select(.active == true) | .name' | head -n 1)"
  fi
  if [[ -n "$ACTIVE_PROFILE" ]]; then
    run_success_case "auth_use" cli auth use "$ACTIVE_PROFILE"
  else
    skip_case "auth_use" "no active profile reported by auth list"
  fi
  run_text_case "yaml_success" '^active_profile:' cli --output yaml auth doctor
  run_text_error_case "yaml_error" '^object: error$' cli --output yaml page get 00000000-0000-0000-0000-000000000000
}

run_user_cases() {
  run_success_case "user_me" cli user me
  local user_id=""
  if [[ $LAST_STATUS -eq 0 ]]; then
    user_id="$(json_value '.id // empty')"
  fi

  run_success_case "user_list_page1" cli user list --page-size 1
  if [[ -z "$user_id" && $LAST_STATUS -eq 0 ]]; then
    user_id="$(json_value '.results[0].id // empty')"
  fi

  if [[ -n "$user_id" ]]; then
    run_success_case "user_get" cli user get "$user_id"
  else
    skip_case "user_get" "no user id available"
  fi
}

run_negative_cases() {
  run_error_case "not_found_page" "not_found" cli page get 00000000-0000-0000-0000-000000000000
  run_error_case "malformed_json_body" "validation_error" cli data-source create --body-json '{'
  run_error_case "missing_json_body" "validation_error" cli data-source create
  run_any_error_case "oversized_upload" cli file-upload create --file "$TMPDIR/too-large.bin" --content-type application/octet-stream
}

run_page_and_block_cases() {
  if [[ -z "$ROOT_PARENT_ID" ]]; then
    skip_case "container_page_create" "no shared parent page available"
    return
  fi

  run_success_case "container_page_create" cli page create --parent-page "$ROOT_PARENT_ID" --title "$PREFIX-container" --from-file "$TMPDIR/content.md"
  if [[ $LAST_STATUS -eq 0 ]]; then
    CONTAINER_PAGE_ID="$(json_value '.id // empty')"
  fi

  if [[ -z "$CONTAINER_PAGE_ID" ]]; then
    skip_case "content_page_create" "container page create failed"
    return
  fi

  run_success_case "content_page_create" cli page create --parent-page "$CONTAINER_PAGE_ID" --title "$PREFIX-content" --from-file "$TMPDIR/content.md"
  if [[ $LAST_STATUS -eq 0 ]]; then
    CONTENT_PAGE_ID="$(json_value '.id // empty')"
  fi

  if [[ -z "$CONTENT_PAGE_ID" ]]; then
    skip_case "page_get" "content page create failed"
    skip_case "page_property" "content page create failed"
    skip_case "page_update" "content page create failed"
    skip_case "page_append" "content page create failed"
    skip_case "page_replace" "content page create failed"
    skip_case "block_children_initial" "content page create failed"
    skip_case "block_append_batch" "content page create failed"
    skip_case "block_children_paged" "content page create failed"
    skip_case "block_get" "content page create failed"
    skip_case "block_update" "content page create failed"
    skip_case "block_delete" "content page create failed"
    return
  fi

  run_success_case "search_content_page" cli search "$PREFIX-content" --type page --limit 5
  run_success_case "page_get" cli page get "$CONTENT_PAGE_ID" --include-markdown
  run_success_case "page_property" cli page property "$CONTENT_PAGE_ID" title
  run_success_case "page_update" cli page update "$CONTENT_PAGE_ID" --body-json '{"cover":{"type":"external","external":{"url":"https://example.com/cover.png"}}}'
  run_success_case "page_append" cli page append "$CONTENT_PAGE_ID" --from-file "$TMPDIR/append.md"
  run_success_case "page_replace" cli page replace "$CONTENT_PAGE_ID" --from-file "$TMPDIR/replace.md" --allow-deleting-content
  run_success_case "block_children_initial" cli block children "$CONTENT_PAGE_ID"
  run_success_case "block_append_batch" cli block append "$CONTENT_PAGE_ID" --body-json '{"children":[{"object":"block","type":"paragraph","paragraph":{"rich_text":[{"type":"text","text":{"content":"Block smoke A"}}]}},{"object":"block","type":"paragraph","paragraph":{"rich_text":[{"type":"text","text":{"content":"Block smoke B"}}]}}]}'
  run_success_case "block_children_after_append" cli block children "$CONTENT_PAGE_ID"
  if [[ $LAST_STATUS -eq 0 ]]; then
    BLOCK_ID="$(json_value '.results[] | select(.type == "paragraph") | .id' | tail -n 1)"
  fi
  run_success_case "block_children_paged" cli block children "$CONTENT_PAGE_ID" --page-size 1

  if [[ -n "$BLOCK_ID" ]]; then
    run_success_case "block_get" cli block get "$BLOCK_ID"
    run_success_case "block_update" cli block update "$BLOCK_ID" --body-json '{"paragraph":{"rich_text":[{"type":"text","text":{"content":"Updated block smoke"}}]}}'
    run_success_case "block_delete" cli block delete "$BLOCK_ID"
  else
    skip_case "block_get" "no block id available"
    skip_case "block_update" "no block id available"
    skip_case "block_delete" "no block id available"
  fi

  run_success_case "trash_page_create" cli page create --parent-page "$CONTAINER_PAGE_ID" --title "$PREFIX-trash" --from-file "$TMPDIR/content.md"
  if [[ $LAST_STATUS -eq 0 ]]; then
    TRASH_PAGE_ID="$(json_value '.id // empty')"
  fi
  if [[ -n "$TRASH_PAGE_ID" ]]; then
    run_success_case "page_trash" cli page trash "$TRASH_PAGE_ID"
    run_success_case "page_restore" cli page restore "$TRASH_PAGE_ID"
  else
    skip_case "page_trash" "trash target create failed"
    skip_case "page_restore" "trash target create failed"
  fi
}

run_comment_cases() {
  if [[ -z "$CONTENT_PAGE_ID" ]]; then
    skip_case "comment_list_empty" "content page missing"
    skip_case "comment_create_1" "content page missing"
    skip_case "comment_create_2" "content page missing"
    skip_case "comment_list_paged" "content page missing"
    skip_case "comment_get" "content page missing"
    return
  fi

  run_success_case "comment_list_empty" cli comment list "$CONTENT_PAGE_ID"
  run_success_case "comment_create_1" cli comment create --body-json "{\"parent\":{\"page_id\":\"$CONTENT_PAGE_ID\"},\"rich_text\":[{\"type\":\"text\",\"text\":{\"content\":\"$PREFIX comment 1\"}}]}"
  if [[ $LAST_STATUS -eq 0 ]]; then
    COMMENT_ID="$(json_value '.id // empty')"
  fi
  run_success_case "comment_create_2" cli comment create --body-json "{\"parent\":{\"page_id\":\"$CONTENT_PAGE_ID\"},\"rich_text\":[{\"type\":\"text\",\"text\":{\"content\":\"$PREFIX comment 2\"}}]}"
  if [[ $LAST_STATUS -eq 0 ]]; then
    COMMENT_ID_2="$(json_value '.id // empty')"
  fi
  run_success_case "comment_list_paged" cli comment list "$CONTENT_PAGE_ID" --page-size 1
  if [[ -n "$COMMENT_ID" ]]; then
    run_success_case "comment_get" cli comment get "$COMMENT_ID"
  else
    skip_case "comment_get" "no comment id available"
  fi
}

run_database_and_data_source_cases() {
  if [[ -z "$CONTAINER_PAGE_ID" ]]; then
    skip_case "database_create" "container page missing"
    skip_case "database_get" "container page missing"
    skip_case "database_update" "container page missing"
    skip_case "data_source_get" "container page missing"
    skip_case "data_source_query_empty" "container page missing"
    skip_case "data_source_row_create_1" "container page missing"
    skip_case "data_source_row_create_2" "container page missing"
    skip_case "data_source_query_paged" "container page missing"
    skip_case "data_source_create" "container page missing"
    skip_case "data_source_update" "container page missing"
    return
  fi

  run_success_case "database_create" cli database create --body-json "{\"parent\":{\"type\":\"page_id\",\"page_id\":\"$CONTAINER_PAGE_ID\"},\"title\":[{\"type\":\"text\",\"text\":{\"content\":\"$PREFIX database\"}}],\"initial_data_source\":{\"title\":[{\"type\":\"text\",\"text\":{\"content\":\"Primary\"}}],\"properties\":{\"Name\":{\"title\":{}},\"Notes\":{\"rich_text\":{}}}}}"
  if [[ $LAST_STATUS -eq 0 ]]; then
    DATABASE_ID="$(json_value '.id // empty')"
    DATA_SOURCE_ID="$(json_value '.data_sources[0].id // .initial_data_source.id // empty')"
  fi

  if [[ -z "$DATABASE_ID" || -z "$DATA_SOURCE_ID" ]]; then
    skip_case "database_get" "database create failed"
    skip_case "database_update" "database create failed"
    skip_case "data_source_get" "database create failed"
    skip_case "data_source_query_empty" "database create failed"
    skip_case "data_source_row_create_1" "database create failed"
    skip_case "data_source_row_create_2" "database create failed"
    skip_case "data_source_query_paged" "database create failed"
    skip_case "data_source_create" "database create failed"
    skip_case "data_source_update" "database create failed"
    return
  fi

  run_success_case "database_get" cli database get "$DATABASE_ID"
  run_success_case "database_update" cli database update "$DATABASE_ID" --body-json "{\"title\":[{\"type\":\"text\",\"text\":{\"content\":\"$PREFIX database updated\"}}]}"
  run_success_case "data_source_get" cli data-source get "$DATA_SOURCE_ID"
  run_success_case "data_source_query_empty" cli data-source query "$DATA_SOURCE_ID"
  run_success_case "data_source_row_create_1" cli page create --parent-data-source "$DATA_SOURCE_ID" --title "$PREFIX row 1" --from-file "$TMPDIR/content.md"
  if [[ $LAST_STATUS -eq 0 ]]; then
    ROW_ID_1="$(json_value '.id // empty')"
  fi
  run_success_case "data_source_row_create_2" cli page create --parent-data-source "$DATA_SOURCE_ID" --title "$PREFIX row 2" --from-file "$TMPDIR/content.md"
  if [[ $LAST_STATUS -eq 0 ]]; then
    ROW_ID_2="$(json_value '.id // empty')"
  fi
  run_success_case "data_source_query_paged" cli data-source query "$DATA_SOURCE_ID" --page-size 1
  run_success_case "data_source_create" cli data-source create --body-json "{\"parent\":{\"type\":\"database_id\",\"database_id\":\"$DATABASE_ID\"},\"title\":[{\"type\":\"text\",\"text\":{\"content\":\"$PREFIX secondary\"}}],\"properties\":{\"Name\":{\"title\":{}},\"Notes\":{\"rich_text\":{}}}}"
  if [[ $LAST_STATUS -eq 0 ]]; then
    SECONDARY_DATA_SOURCE_ID="$(json_value '.id // empty')"
  fi

  if [[ -n "$SECONDARY_DATA_SOURCE_ID" ]]; then
    run_success_case "data_source_update" cli data-source update "$SECONDARY_DATA_SOURCE_ID" --body-json "{\"title\":[{\"type\":\"text\",\"text\":{\"content\":\"$PREFIX secondary updated\"}}]}"
  else
    skip_case "data_source_update" "secondary data source create failed"
  fi
}

run_file_upload_cases() {
  run_success_case "file_upload_list_before" cli file-upload list --page-size 5
  run_success_case "file_upload_create" cli file-upload create --file "$TMPDIR/upload.txt" --content-type text/plain
  if [[ $LAST_STATUS -eq 0 ]]; then
    FILE_UPLOAD_ID="$(json_value '.id // empty')"
  fi
  if [[ -n "$FILE_UPLOAD_ID" ]]; then
    run_success_case "file_upload_get" cli file-upload get "$FILE_UPLOAD_ID"
    run_success_case "file_upload_list_after" cli file-upload list --page-size 1
  else
    skip_case "file_upload_get" "file upload create failed"
    skip_case "file_upload_list_after" "file upload create failed"
  fi
}

cleanup_artifacts() {
  if [[ "$KEEP_ARTIFACTS" == "1" || -z "$CONTAINER_PAGE_ID" ]]; then
    return
  fi

  run_success_case "cleanup_container_trash" cli page trash "$CONTAINER_PAGE_ID"
}

print_report() {
  printf 'REPORT\n'
  if command -v column >/dev/null 2>&1; then
    column -t -s $'\t' "$REPORT_TSV"
  else
    cat "$REPORT_TSV"
  fi

  printf '\nCOUNTS\n'
  awk -F '\t' '{counts[$1]++} END {for (key in counts) printf "%s=%d\n", key, counts[key]}' "$REPORT_TSV" | sort

  printf '\nARTIFACTS\n'
  printf 'tmpdir=%s\n' "$TMPDIR"
  printf 'root_parent_id=%s\n' "$ROOT_PARENT_ID"
  printf 'container_page_id=%s\n' "$CONTAINER_PAGE_ID"
  printf 'content_page_id=%s\n' "$CONTENT_PAGE_ID"
  printf 'trash_page_id=%s\n' "$TRASH_PAGE_ID"
  printf 'block_id=%s\n' "$BLOCK_ID"
  printf 'comment_id=%s\n' "$COMMENT_ID"
  printf 'comment_id_2=%s\n' "$COMMENT_ID_2"
  printf 'database_id=%s\n' "$DATABASE_ID"
  printf 'data_source_id=%s\n' "$DATA_SOURCE_ID"
  printf 'row_id_1=%s\n' "$ROW_ID_1"
  printf 'row_id_2=%s\n' "$ROW_ID_2"
  printf 'secondary_data_source_id=%s\n' "$SECONDARY_DATA_SOURCE_ID"
  printf 'file_upload_id=%s\n' "$FILE_UPLOAD_ID"
}

main() {
  require_command jq
  require_command dd
  ensure_binary
  write_fixture_files

  run_core_auth_and_output_cases
  discover_parent
  run_user_cases
  run_negative_cases
  run_page_and_block_cases

  if [[ "$SCOPE" != "smoke" ]]; then
    run_comment_cases
    run_database_and_data_source_cases
    run_file_upload_cases
  fi

  cleanup_artifacts
  print_report
}

main "$@"
