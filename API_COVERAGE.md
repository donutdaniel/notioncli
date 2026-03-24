# Notion API Coverage

This CLI targets Notion API version `2026-03-11` and focuses on the parts of
the official API that are practical for a local structured CLI.

## Implemented

### Authentication and user identity

- `GET /v1/users/me`
  - `notioncli auth whoami`
  - `notioncli auth doctor`
  - `notioncli user me`
- `GET /v1/users`
  - `notioncli user list`
- `GET /v1/users/{user_id}`
  - `notioncli user get <user-id>`

### Search

- `POST /v1/search`
  - `notioncli search "<query>" --type all|page|data-source`

### Pages

- `GET /v1/pages/{page_id}`
  - `notioncli page get <page-id-or-url>`
- `GET /v1/pages/{page_id}/properties/{property_id}`
  - `notioncli page property <page-id-or-url> <property-id>`
- `GET /v1/pages/{page_id}/markdown`
  - `notioncli page get <page-id-or-url> --include-markdown`
- `POST /v1/pages`
  - `notioncli page create --parent-page <page-id> --title <title>`
  - `notioncli page create --parent-data-source <data-source-id> --title <title>`
  - `notioncli page create --parent <page-or-data-source> --title <title>`
- `PATCH /v1/pages/{page_id}`
  - `notioncli page update <page-id-or-url> --body-json ...`
  - `notioncli page trash <page-id-or-url>`
  - `notioncli page restore <page-id-or-url>`
- `PATCH /v1/pages/{page_id}/markdown`
  - `notioncli page append <page-id-or-url> --stdin|--from-file`
  - `notioncli page replace <page-id-or-url> --stdin|--from-file`

### Blocks

- `GET /v1/blocks/{block_id}`
  - `notioncli block get <block-id-or-url>`
- `GET /v1/blocks/{block_id}/children`
  - `notioncli block children <block-id-or-url>`
- `PATCH /v1/blocks/{block_id}/children`
  - `notioncli block append <block-id-or-url> --body-json ...`
- `PATCH /v1/blocks/{block_id}`
  - `notioncli block update <block-id-or-url> --body-json ...`
- `DELETE /v1/blocks/{block_id}`
  - `notioncli block delete <block-id-or-url>`

### Databases and data sources

- `GET /v1/databases/{database_id}`
  - `notioncli database get <database-id-or-url>`
- `POST /v1/databases`
  - `notioncli database create --body-json ...`
- `PATCH /v1/databases/{database_id}`
  - `notioncli database update <database-id-or-url> --body-json ...`
- `GET /v1/data_sources/{data_source_id}`
  - `notioncli data-source get <data-source-id-or-url>`
- `POST /v1/data_sources`
  - `notioncli data-source create --body-json ...`
- `PATCH /v1/data_sources/{data_source_id}`
  - `notioncli data-source update <data-source-id-or-url> --body-json ...`
- `POST /v1/data_sources/{data_source_id}/query`
  - `notioncli data-source query <data-source-id-or-url>`

### Comments

- `GET /v1/comments`
  - `notioncli comment list <page-or-block-id>`
- `GET /v1/comments/{comment_id}`
  - `notioncli comment get <comment-id>`
- `POST /v1/comments`
  - `notioncli comment create --body-json ...`

### File uploads

- `GET /v1/file_uploads`
  - `notioncli file-upload list`
- `GET /v1/file_uploads/{file_upload_id}`
  - `notioncli file-upload get <file-upload-id>`
- `POST /v1/file_uploads`
  - internal step of `notioncli file-upload create --file <path>`
- `POST /v1/file_uploads/{file_upload_id}/send`
  - internal step of `notioncli file-upload create --file <path>`

## Partial or intentionally narrow

### File uploads

The CLI currently implements direct single-part uploads for local files. It does
not yet expose:

- multi-part uploads for files larger than 20MB
- external URL imports
- explicit `complete` flow for multi-part uploads

### Complex mutable schemas

For several mutable endpoints, the CLI takes raw JSON request bodies instead of
trying to wrap every Notion schema variant in dedicated flags. This is
intentional for:

- `page update`
- `block append`
- `block update`
- `comment create`
- `database create`
- `database update`
- `data-source create`
- `data-source update`

### Legacy database APIs

The CLI uses the current `database` container APIs and `data_source` APIs from
`2025-09-03+`. It does not implement deprecated database listing/querying
endpoints from older API versions.

## Not implemented because they are not a good fit here

### Webhooks

Webhooks require a secure public endpoint. A local standalone CLI cannot receive
Notion webhooks directly without adding a public relay or tunnel.

### Views management

Notion currently does not support managing views through the public REST API.

### Workspace creation

The public API operates inside existing workspaces. It does not expose a normal
workspace creation flow for this CLI to automate.

## Good next additions

- multi-part and external-URL file uploads
- richer page-property helpers for common property types
- stronger block/body validators on top of the raw JSON entry points
- coverage tests for the newly added commands with mocked HTTP responses
- CI-friendly live test automation on top of `scripts/live_smoke.sh` and
  `scripts/live_matrix.sh`

## Sources

- https://developers.notion.com/reference
- https://developers.notion.com/reference/changes-by-version
- https://developers.notion.com/reference/update-a-data-source
- https://developers.notion.com/reference/webhooks
- https://developers.notion.com/reference/send-a-file-upload
- https://developers.notion.com/reference/create-a-file-upload
- https://developers.notion.com/reference/list-file-uploads
- https://developers.notion.com/reference/retrieve-a-file-upload
