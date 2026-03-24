use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use reqwest::multipart::{Form, Part};
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::sleep;
use tracing::debug;

static DASHED_UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap()
});
static COMPACT_UUID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[0-9a-f]{32}").unwrap());

use crate::config::{ConfigStore, DEFAULT_API_BASE_URL, RuntimeSession};

#[derive(Debug, Clone)]
pub struct NotionClient {
    http: reqwest::Client,
    base_url: String,
    notion_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    #[serde(default = "default_list_object")]
    pub object: String,
    pub results: Vec<Value>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(rename = "type", default)]
    pub list_type: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

fn default_list_object() -> String {
    "list".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageMarkdown {
    pub object: String,
    pub markdown: String,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub unknown_block_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum SearchFilter {
    All,
    Page,
    DataSource,
}

impl SearchFilter {
    fn api_value(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Page => Some("page"),
            Self::DataSource => Some("data_source"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CreateParent {
    Page {
        page_id: String,
    },
    DataSource {
        data_source_id: String,
        title_property: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DataSourceQuery {
    pub filter: Option<Value>,
    pub sorts: Option<Value>,
    pub page_size: Option<usize>,
    pub cursor: Option<String>,
}

impl NotionClient {
    pub fn new(base_url: impl Into<String>, notion_version: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(format!("notioncli/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .context("failed to create HTTP client")?;

        Ok(Self {
            http,
            base_url: normalize_base_url(&base_url.into()),
            notion_version: notion_version.into(),
        })
    }

    pub fn from_config(store: &ConfigStore) -> Result<Self> {
        Self::new(store.api_base_url(), store.api_version())
    }

    pub async fn search(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        query: &str,
        filter: SearchFilter,
        limit: usize,
    ) -> Result<ListResponse> {
        let mut all_results = Vec::new();
        let mut next_cursor = None;
        let mut has_more = false;
        let mut object = default_list_object();
        let mut list_type = None;
        let mut extra = BTreeMap::new();

        while all_results.len() < limit {
            let page_size = std::cmp::min(100, limit.saturating_sub(all_results.len()));
            let mut body = json!({
                "query": query,
                "page_size": page_size,
            });

            if let Some(value) = filter.api_value() {
                body["filter"] = json!({
                    "property": "object",
                    "value": value,
                });
            }

            if let Some(cursor) = &next_cursor {
                body["start_cursor"] = json!(cursor);
            }

            let page: ListResponse = serde_json::from_value(
                self.request_json(session, store, Method::POST, "/v1/search", None, Some(body))
                    .await?,
            )?;

            if all_results.is_empty() {
                object = page.object.clone();
                list_type = page.list_type.clone();
                extra = page.extra.clone();
            }

            all_results.extend(page.results);
            has_more = page.has_more;
            next_cursor = page.next_cursor;

            if !has_more || next_cursor.is_none() {
                break;
            }
        }

        // If we stopped because we hit the limit but the server has more,
        // reflect that in the response.
        if all_results.len() >= limit && has_more {
            // Truncate to exact limit; keep next_cursor from last page.
        } else if !has_more {
            next_cursor = None;
        }

        Ok(ListResponse {
            object,
            results: all_results,
            next_cursor,
            has_more,
            list_type,
            extra,
        })
    }

    pub async fn get_self(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
    ) -> Result<Value> {
        self.request_json(session, store, Method::GET, "/v1/users/me", None, None)
            .await
    }

    pub async fn get_self_for_token(&self, token: &str) -> Result<Value> {
        let response = self
            .request_bearer_json(token, Method::GET, "/v1/users/me", None, None)
            .await?;
        decode_response(response).await
    }

    pub async fn get_page(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        page_id_or_url: &str,
    ) -> Result<Value> {
        let page_id = normalize_notion_id(page_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/pages/{page_id}"),
            None,
            None,
        )
        .await
    }

    pub async fn get_page_property(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        page_id_or_url: &str,
        property_id: &str,
        page_size: Option<usize>,
        cursor: Option<String>,
    ) -> Result<Value> {
        let page_id = normalize_notion_id(page_id_or_url)?;
        let mut query = Vec::new();
        if let Some(value) = page_size {
            query.push(("page_size".to_string(), value.to_string()));
        }
        if let Some(value) = cursor {
            query.push(("start_cursor".to_string(), value));
        }

        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/pages/{page_id}/properties/{property_id}"),
            Some(query),
            None,
        )
        .await
    }

    pub async fn get_page_markdown(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        page_id_or_url: &str,
        include_transcript: bool,
    ) -> Result<PageMarkdown> {
        let page_id = normalize_notion_id(page_id_or_url)?;
        let query = if include_transcript {
            vec![("include_transcript".to_string(), "true".to_string())]
        } else {
            Vec::new()
        };

        let response = self
            .request_json(
                session,
                store,
                Method::GET,
                &format!("/v1/pages/{page_id}/markdown"),
                Some(query),
                None,
            )
            .await?;

        serde_json::from_value(response).context("failed to decode markdown response")
    }

    pub async fn get_data_source(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        data_source_id_or_url: &str,
    ) -> Result<Value> {
        let id = normalize_notion_id(data_source_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/data_sources/{id}"),
            None,
            None,
        )
        .await
    }

    pub async fn create_data_source(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        body: Value,
    ) -> Result<Value> {
        self.request_json(
            session,
            store,
            Method::POST,
            "/v1/data_sources",
            None,
            Some(body),
        )
        .await
    }

    pub async fn update_data_source(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        data_source_id_or_url: &str,
        body: Value,
    ) -> Result<Value> {
        let id = normalize_notion_id(data_source_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::PATCH,
            &format!("/v1/data_sources/{id}"),
            None,
            Some(body),
        )
        .await
    }

    pub async fn query_data_source(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        data_source_id_or_url: &str,
        query: DataSourceQuery,
    ) -> Result<ListResponse> {
        let id = normalize_notion_id(data_source_id_or_url)?;
        let mut body = json!({});

        if let Some(value) = query.filter {
            body["filter"] = value;
        }

        if let Some(value) = query.sorts {
            body["sorts"] = value;
        }

        if let Some(value) = query.page_size {
            body["page_size"] = json!(value);
        }

        if let Some(value) = query.cursor {
            body["start_cursor"] = json!(value);
        }

        serde_json::from_value(
            self.request_json(
                session,
                store,
                Method::POST,
                &format!("/v1/data_sources/{id}/query"),
                None,
                Some(body),
            )
            .await?,
        )
        .context("failed to decode data source query response")
    }

    pub async fn create_page(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        parent: CreateParent,
        title: &str,
        markdown: Option<String>,
    ) -> Result<Value> {
        let parent_json = match parent {
            CreateParent::Page { page_id } => json!({ "type": "page_id", "page_id": page_id }),
            CreateParent::DataSource {
                data_source_id,
                title_property,
            } => {
                let mut properties = serde_json::Map::new();
                properties.insert(
                    title_property,
                    json!({
                        "title": [
                            { "text": { "content": title } }
                        ]
                    }),
                );

                let mut body = serde_json::Map::new();
                body.insert(
                    "parent".into(),
                    json!({
                        "type": "data_source_id",
                        "data_source_id": data_source_id,
                    }),
                );
                body.insert("properties".into(), Value::Object(properties));

                if let Some(markdown) = markdown {
                    body.insert("markdown".into(), json!(markdown));
                }

                return self
                    .request_json(
                        session,
                        store,
                        Method::POST,
                        "/v1/pages",
                        None,
                        Some(Value::Object(body)),
                    )
                    .await;
            }
        };

        let body = match markdown {
            Some(markdown) => json!({
                "parent": parent_json,
                "properties": {
                    "title": {
                        "title": [
                            { "text": { "content": title } }
                        ]
                    }
                },
                "markdown": markdown,
            }),
            None => json!({
                "parent": parent_json,
                "properties": {
                    "title": {
                        "title": [
                            { "text": { "content": title } }
                        ]
                    }
                }
            }),
        };

        self.request_json(session, store, Method::POST, "/v1/pages", None, Some(body))
            .await
    }

    pub async fn update_page(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        page_id_or_url: &str,
        body: Value,
    ) -> Result<Value> {
        let page_id = normalize_notion_id(page_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::PATCH,
            &format!("/v1/pages/{page_id}"),
            None,
            Some(body),
        )
        .await
    }

    pub async fn get_database(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        database_id_or_url: &str,
    ) -> Result<Value> {
        let database_id = normalize_notion_id(database_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/databases/{database_id}"),
            None,
            None,
        )
        .await
    }

    pub async fn create_database(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        body: Value,
    ) -> Result<Value> {
        self.request_json(
            session,
            store,
            Method::POST,
            "/v1/databases",
            None,
            Some(body),
        )
        .await
    }

    pub async fn update_database(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        database_id_or_url: &str,
        body: Value,
    ) -> Result<Value> {
        let database_id = normalize_notion_id(database_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::PATCH,
            &format!("/v1/databases/{database_id}"),
            None,
            Some(body),
        )
        .await
    }

    pub async fn append_page_markdown(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        page_id_or_url: &str,
        markdown: String,
    ) -> Result<PageMarkdown> {
        let page_id = normalize_notion_id(page_id_or_url)?;
        let body = json!({
            "type": "insert_content",
            "insert_content": {
                "content": markdown
            }
        });

        serde_json::from_value(
            self.request_json(
                session,
                store,
                Method::PATCH,
                &format!("/v1/pages/{page_id}/markdown"),
                None,
                Some(body),
            )
            .await?,
        )
        .context("failed to decode markdown append response")
    }

    pub async fn replace_page_markdown(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        page_id_or_url: &str,
        markdown: String,
        allow_deleting_content: bool,
    ) -> Result<PageMarkdown> {
        let page_id = normalize_notion_id(page_id_or_url)?;
        let body = json!({
            "type": "replace_content",
            "replace_content": {
                "new_str": markdown,
                "allow_deleting_content": allow_deleting_content
            }
        });

        serde_json::from_value(
            self.request_json(
                session,
                store,
                Method::PATCH,
                &format!("/v1/pages/{page_id}/markdown"),
                None,
                Some(body),
            )
            .await?,
        )
        .context("failed to decode markdown replace response")
    }

    pub async fn resolve_create_parent(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        parent_id_or_url: &str,
    ) -> Result<CreateParent> {
        let normalized = normalize_notion_id(parent_id_or_url)?;

        if let Ok(parent) = self
            .resolve_data_source_parent(session, store, &normalized, None)
            .await
        {
            return Ok(parent);
        }

        let _page = self
            .request_json(
                session,
                store,
                Method::GET,
                &format!("/v1/pages/{normalized}"),
                None,
                None,
            )
            .await?;

        Ok(CreateParent::Page {
            page_id: normalized,
        })
    }

    pub async fn resolve_data_source_parent(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        data_source_id_or_url: &str,
        title_property_override: Option<String>,
    ) -> Result<CreateParent> {
        let normalized = normalize_notion_id(data_source_id_or_url)?;

        if let Some(title_property) = title_property_override {
            return Ok(CreateParent::DataSource {
                data_source_id: normalized,
                title_property,
            });
        }

        let data_source = self
            .request_json(
                session,
                store,
                Method::GET,
                &format!("/v1/data_sources/{normalized}"),
                None,
                None,
            )
            .await?;

        let title_property = data_source_title_property_name(&data_source).ok_or_else(|| {
            anyhow!("could not determine the title property for data source `{normalized}`")
        })?;

        Ok(CreateParent::DataSource {
            data_source_id: normalized,
            title_property,
        })
    }

    pub async fn get_block(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        block_id_or_url: &str,
    ) -> Result<Value> {
        let block_id = normalize_notion_id(block_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/blocks/{block_id}"),
            None,
            None,
        )
        .await
    }

    pub async fn get_block_children(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        block_id_or_url: &str,
        page_size: Option<usize>,
        cursor: Option<String>,
    ) -> Result<ListResponse> {
        let block_id = normalize_notion_id(block_id_or_url)?;
        let mut query = Vec::new();
        if let Some(value) = page_size {
            query.push(("page_size".to_string(), value.to_string()));
        }
        if let Some(value) = cursor {
            query.push(("start_cursor".to_string(), value));
        }

        serde_json::from_value(
            self.request_json(
                session,
                store,
                Method::GET,
                &format!("/v1/blocks/{block_id}/children"),
                Some(query),
                None,
            )
            .await?,
        )
        .context("failed to decode block children response")
    }

    pub async fn append_block_children(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        block_id_or_url: &str,
        children: Value,
        position: Option<Value>,
    ) -> Result<Value> {
        let block_id = normalize_notion_id(block_id_or_url)?;
        let mut body = json!({
            "children": children,
        });
        if let Some(position) = position {
            body["position"] = position;
        }

        self.request_json(
            session,
            store,
            Method::PATCH,
            &format!("/v1/blocks/{block_id}/children"),
            None,
            Some(body),
        )
        .await
    }

    pub async fn update_block(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        block_id_or_url: &str,
        body: Value,
    ) -> Result<Value> {
        let block_id = normalize_notion_id(block_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::PATCH,
            &format!("/v1/blocks/{block_id}"),
            None,
            Some(body),
        )
        .await
    }

    pub async fn delete_block(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        block_id_or_url: &str,
    ) -> Result<Value> {
        let block_id = normalize_notion_id(block_id_or_url)?;
        self.request_json(
            session,
            store,
            Method::DELETE,
            &format!("/v1/blocks/{block_id}"),
            None,
            None,
        )
        .await
    }

    pub async fn list_users(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        page_size: Option<usize>,
        cursor: Option<String>,
    ) -> Result<ListResponse> {
        let mut query = Vec::new();
        if let Some(value) = page_size {
            query.push(("page_size".to_string(), value.to_string()));
        }
        if let Some(value) = cursor {
            query.push(("start_cursor".to_string(), value));
        }

        serde_json::from_value(
            self.request_json(session, store, Method::GET, "/v1/users", Some(query), None)
                .await?,
        )
        .context("failed to decode users list response")
    }

    pub async fn get_user(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        user_id: &str,
    ) -> Result<Value> {
        let user_id = normalize_notion_id(user_id)?;
        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/users/{user_id}"),
            None,
            None,
        )
        .await
    }

    pub async fn list_comments(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        block_id_or_page_id: &str,
        page_size: Option<usize>,
        cursor: Option<String>,
    ) -> Result<ListResponse> {
        let mut query = Vec::new();
        query.push((
            "block_id".to_string(),
            normalize_notion_id(block_id_or_page_id)?,
        ));
        if let Some(value) = page_size {
            query.push(("page_size".to_string(), value.to_string()));
        }
        if let Some(value) = cursor {
            query.push(("start_cursor".to_string(), value));
        }

        serde_json::from_value(
            self.request_json(
                session,
                store,
                Method::GET,
                "/v1/comments",
                Some(query),
                None,
            )
            .await?,
        )
        .context("failed to decode comments list response")
    }

    pub async fn get_comment(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        comment_id: &str,
    ) -> Result<Value> {
        let comment_id = normalize_notion_id(comment_id)?;
        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/comments/{comment_id}"),
            None,
            None,
        )
        .await
    }

    pub async fn create_comment(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        body: Value,
    ) -> Result<Value> {
        self.request_json(
            session,
            store,
            Method::POST,
            "/v1/comments",
            None,
            Some(body),
        )
        .await
    }

    pub async fn list_file_uploads(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        status: Option<&str>,
        page_size: Option<usize>,
        cursor: Option<String>,
    ) -> Result<ListResponse> {
        let mut query = Vec::new();
        if let Some(value) = status {
            query.push(("status".to_string(), value.to_string()));
        }
        if let Some(value) = page_size {
            query.push(("page_size".to_string(), value.to_string()));
        }
        if let Some(value) = cursor {
            query.push(("start_cursor".to_string(), value));
        }

        serde_json::from_value(
            self.request_json(
                session,
                store,
                Method::GET,
                "/v1/file_uploads",
                Some(query),
                None,
            )
            .await?,
        )
        .context("failed to decode file uploads list response")
    }

    pub async fn get_file_upload(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        file_upload_id: &str,
    ) -> Result<Value> {
        let file_upload_id = normalize_notion_id(file_upload_id)?;
        self.request_json(
            session,
            store,
            Method::GET,
            &format!("/v1/file_uploads/{file_upload_id}"),
            None,
            None,
        )
        .await
    }

    pub async fn upload_small_file(
        &self,
        session: &mut RuntimeSession,
        store: &mut ConfigStore,
        filename: &str,
        content_type: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<Value> {
        let mut create_body = json!({
            "mode": "single_part",
            "filename": filename,
        });
        if let Some(value) = content_type {
            create_body["content_type"] = json!(value);
        }

        let created = self
            .request_json(
                session,
                store,
                Method::POST,
                "/v1/file_uploads",
                None,
                Some(create_body),
            )
            .await?;
        let file_upload_id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("file upload creation response did not include an id"))?;

        let inferred_content_type = content_type.unwrap_or("application/octet-stream");
        let part = Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(inferred_content_type)
            .with_context(|| {
                format!("invalid content type `{inferred_content_type}` for file upload")
            })?;
        let form = Form::new().part("file", part);

        let send_url = format!("{}/v1/file_uploads/{file_upload_id}/send", self.base_url);
        let response = self
            .http
            .post(send_url)
            .header("Notion-Version", &self.notion_version)
            .header("Accept", "application/json")
            .bearer_auth(session.secret.access_token())
            .multipart(form)
            .send()
            .await
            .context("sending file upload to Notion failed")?;

        let _ = decode_response(response).await?;
        self.get_file_upload(session, store, file_upload_id).await
    }

    async fn request_json(
        &self,
        session: &mut RuntimeSession,
        _store: &mut ConfigStore,
        method: Method,
        path: &str,
        query: Option<Vec<(String, String)>>,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut attempts = 0usize;

        loop {
            attempts += 1;

            let response = self
                .request_bearer_json(
                    session.secret.access_token(),
                    method.clone(),
                    path,
                    query.clone(),
                    body.clone(),
                )
                .await?;

            if should_retry(response.status()) && attempts < 4 {
                let delay = retry_delay(&response, attempts);
                debug!("retrying {} {} in {:?}", method, path, delay);
                sleep(delay).await;
                continue;
            }

            return decode_response(response).await;
        }
    }

    async fn request_bearer_json(
        &self,
        token: &str,
        method: Method,
        path: &str,
        query: Option<Vec<(String, String)>>,
        body: Option<Value>,
    ) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self
            .http
            .request(method, url)
            .header("Notion-Version", &self.notion_version)
            .header("Accept", "application/json")
            .bearer_auth(token);

        if let Some(items) = &query {
            request = request.query(items);
        }

        if let Some(payload) = &body {
            request = request.json(payload);
        }

        request.send().await.context("request to Notion failed")
    }
}

fn should_retry(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn retry_delay(response: &Response, attempt: usize) -> Duration {
    if let Some(value) = response.headers().get("retry-after")
        && let Ok(raw) = value.to_str()
        && let Ok(seconds) = raw.parse::<u64>()
    {
        return Duration::from_secs(seconds.max(1));
    }

    Duration::from_secs((attempt as u64).min(4))
}

async fn decode_response(response: Response) -> Result<Value> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if status.is_success() {
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        return serde_json::from_str(&body).context("failed to decode JSON response");
    }

    let (code, message) = match serde_json::from_str::<Value>(&body) {
        Ok(value) => (
            value
                .get("code")
                .and_then(Value::as_str)
                .map(str::to_string),
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| body.clone()),
        ),
        Err(_) => (None, body.clone()),
    };

    let friendly = friendly_error(status, code.as_deref(), &message);
    bail!("{friendly}");
}

fn friendly_error(status: StatusCode, code: Option<&str>, message: &str) -> String {
    match (status, code) {
        (StatusCode::FORBIDDEN, _) => {
            format!(
                "Notion rejected the request with 403. Check your integration capabilities and page/data source sharing. {message}"
            )
        }
        (StatusCode::NOT_FOUND, Some("object_not_found")) => {
            format!(
                "Notion could not find that object. The page or data source may not be shared with the integration. {message}"
            )
        }
        (StatusCode::UNAUTHORIZED, _) => {
            format!("Notion authentication failed. Re-run `notioncli auth login ...`. {message}")
        }
        (StatusCode::BAD_REQUEST, Some("validation_error")) => {
            format!("Notion rejected the request as invalid. {message}")
        }
        _ => format!("Notion API error {}: {}", status.as_u16(), message),
    }
}

fn normalize_base_url(base_url: &str) -> String {
    let raw = if base_url.trim().is_empty() {
        DEFAULT_API_BASE_URL
    } else {
        base_url
    };
    raw.trim_end_matches('/').to_string()
}

pub fn normalize_notion_id(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("expected a Notion page or data source id/url");
    }

    if let Some(value) = DASHED_UUID.find_iter(trimmed).last() {
        return Ok(value.as_str().to_ascii_lowercase());
    }

    if let Some(value) = COMPACT_UUID.find_iter(trimmed).last() {
        let compact = value.as_str().to_ascii_lowercase();
        return Ok(format!(
            "{}-{}-{}-{}-{}",
            &compact[0..8],
            &compact[8..12],
            &compact[12..16],
            &compact[16..20],
            &compact[20..32]
        ));
    }

    bail!("`{trimmed}` does not look like a Notion page or data source id/url")
}

pub fn data_source_title_property_name(value: &Value) -> Option<String> {
    value
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| {
            properties.iter().find_map(|(name, property)| {
                if property.get("type").and_then(Value::as_str) == Some("title") {
                    Some(name.clone())
                } else {
                    None
                }
            })
        })
}

pub fn object_kind(value: &Value) -> &str {
    value
        .get("object")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

#[cfg(test)]
pub fn extract_title(value: &Value) -> String {
    if let Some(title) = value
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| {
            properties.values().find_map(|property| {
                if property.get("type").and_then(Value::as_str) == Some("title") {
                    property.get("title").and_then(rich_text_to_plain_text)
                } else {
                    None
                }
            })
        })
    {
        return title;
    }

    if let Some(title) = value
        .get("title")
        .and_then(rich_text_to_plain_text)
        .filter(|title| !title.is_empty())
    {
        return title;
    }

    if let Some(title) = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|title| !title.is_empty())
    {
        return title;
    }

    value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("<untitled>")
        .to_string()
}

#[cfg(test)]
fn rich_text_to_plain_text(value: &Value) -> Option<String> {
    let parts = value.as_array()?.iter().filter_map(|item| {
        item.get("plain_text")
            .and_then(Value::as_str)
            .map(str::to_string)
    });

    let title = parts.collect::<String>();
    if title.is_empty() { None } else { Some(title) }
}

pub fn owner_name_from_user_me(value: &Value) -> Option<String> {
    value
        .get("owner")
        .and_then(Value::as_object)
        .and_then(|owner| owner.get("user"))
        .and_then(Value::as_object)
        .and_then(|user| user.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

pub fn owner_email_from_user_me(value: &Value) -> Option<String> {
    value
        .get("owner")
        .and_then(Value::as_object)
        .and_then(|owner| owner.get("user"))
        .and_then(Value::as_object)
        .and_then(|user| user.get("person"))
        .and_then(Value::as_object)
        .and_then(|person| person.get("email"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("person")
                .and_then(Value::as_object)
                .and_then(|person| person.get("email"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

pub fn bot_id_from_user_me(value: &Value) -> Option<String> {
    value.get("id").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_compact_ids() -> Result<()> {
        assert_eq!(
            normalize_notion_id("https://www.notion.so/Example-1234567890abcdef1234567890abcdef")?,
            "12345678-90ab-cdef-1234-567890abcdef"
        );
        Ok(())
    }

    #[test]
    fn extracts_page_titles() {
        let value = json!({
            "object": "page",
            "id": "1",
            "properties": {
                "Name": {
                    "type": "title",
                    "title": [
                        { "plain_text": "Hello" },
                        { "plain_text": " world" }
                    ]
                }
            }
        });

        assert_eq!(extract_title(&value), "Hello world");
    }
}
