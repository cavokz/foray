use crate::migrate;
use crate::store::{Store, StoreError};
use crate::types::{ItemType, JournalFile, JournalItem, JournalSummary, Pagination};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

// ── Auth ─────────────────────────────────────────────────────────────

enum Auth {
    None,
    ApiKey(String),
    Basic { username: String, password: String },
}

// ── ES document types ────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct EsEvent {
    dataset: String,
}

#[derive(Serialize, Deserialize)]
struct EsEntity {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

#[derive(Serialize)]
struct EsJournalDoc {
    event: EsEvent,
    entity: EsEntity,
    archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Deserialize)]
struct EsJournalSource {
    entity: EsEntity,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    meta: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Serialize)]
struct EsItemDoc<'a> {
    #[serde(rename = "@timestamp")]
    timestamp: &'a DateTime<Utc>,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<&'a Vec<String>>,
    event: EsEvent,
    journal_name: &'a str,
    #[serde(rename = "type")]
    item_type: &'a ItemType,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<&'a HashMap<String, serde_json::Value>>,
}

#[derive(Deserialize)]
struct EsItemSource {
    #[serde(rename = "@timestamp")]
    timestamp: DateTime<Utc>,
    message: String,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(rename = "type")]
    item_type: ItemType,
    #[serde(default)]
    meta: Option<HashMap<String, serde_json::Value>>,
}

// ── ES response types ────────────────────────────────────────────────

#[derive(Deserialize)]
struct EsGetResponse<T> {
    #[allow(dead_code)]
    found: bool,
    #[serde(rename = "_source")]
    source: Option<T>,
}

#[derive(Deserialize)]
struct EsSearchResponse<T> {
    hits: EsHits<T>,
}

#[derive(Deserialize)]
struct EsHits<T> {
    total: EsTotal,
    hits: Vec<EsHit<T>>,
}

#[derive(Deserialize)]
struct EsTotal {
    value: usize,
}

#[derive(Deserialize)]
struct EsHit<T> {
    #[serde(rename = "_id")]
    id: String,
    #[serde(rename = "_source")]
    source: T,
}

#[derive(Deserialize)]
struct EsBulkResponse {
    errors: bool,
    items: Vec<EsBulkItem>,
}

#[derive(Deserialize)]
struct EsBulkItem {
    create: EsBulkResult,
}

#[derive(Deserialize)]
struct EsBulkResult {
    status: u16,
}

// ── ElasticsearchStore ───────────────────────────────────────────────

pub struct ElasticsearchStore {
    client: reqwest::Client,
    /// Full URL including index as the last path segment,
    /// e.g. `https://es.example.com/foray-team`.
    index_url: String,
    auth: Auth,
}

impl std::fmt::Debug for ElasticsearchStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElasticsearchStore")
            .field("index_url", &self.index_url)
            .finish_non_exhaustive()
    }
}

impl ElasticsearchStore {
    pub fn new(
        index_url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self, StoreError> {
        let auth = match (api_key, username, password) {
            (Some(key), None, None) => Auth::ApiKey(key),
            (None, Some(user), Some(pass)) => Auth::Basic {
                username: user,
                password: pass,
            },
            (None, None, None) => Auth::None,
            (None, Some(_), None) | (None, None, Some(_)) => {
                return Err(StoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "elasticsearch store: username and password must be provided together",
                )));
            }
            _ => {
                return Err(StoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "elasticsearch store: api_key and username/password are mutually exclusive",
                )));
            }
        };

        // Validate URL: must parse, no embedded credentials, no query/fragment,
        // and must end with a non-empty path segment (the index name).
        let parsed = reqwest::Url::parse(&index_url).map_err(|e| {
            StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("elasticsearch store: invalid URL: {e}"),
            ))
        })?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "elasticsearch store: embed credentials in api_key/username/password, not in the URL",
            )));
        }
        if parsed.query().is_some() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "elasticsearch store: URL must not contain a query string",
            )));
        }
        if parsed.fragment().is_some() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "elasticsearch store: URL must not contain a fragment",
            )));
        }
        let index_segment_ok = parsed
            .path_segments()
            .and_then(|mut segs| segs.next_back())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if !index_segment_ok {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "elasticsearch store: URL must end with a non-empty index name path segment",
            )));
        }

        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| {
                StoreError::Io(std::io::Error::other(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        Ok(Self {
            client,
            index_url,
            auth,
        })
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let builder = self.client.request(method, url);
        match &self.auth {
            Auth::None => builder,
            Auth::ApiKey(key) => builder.header("Authorization", format!("ApiKey {key}")),
            Auth::Basic { username, password } => builder.basic_auth(username, Some(password)),
        }
    }

    fn doc_url(&self, id: &str) -> String {
        format!("{}/_doc/{}", self.index_url, encode_id(id))
    }

    fn search_url(&self) -> String {
        format!("{}/_search", self.index_url)
    }

    fn update_url(&self, id: &str) -> String {
        format!("{}/_update/{}", self.index_url, encode_id(id))
    }

    fn bulk_url(&self) -> String {
        format!("{}/_bulk", self.index_url)
    }

    fn delete_by_query_url(&self) -> String {
        format!("{}/_delete_by_query", self.index_url)
    }

    /// Fetch and deserialize a journal document, returning `None` if not found.
    async fn fetch_journal(&self, name: &str) -> Result<Option<EsJournalSource>, StoreError> {
        let url = self.doc_url(&format!("journal:{name}"));
        let resp = self
            .request(reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|_| net_error("get journal"))?;

        match resp.status() {
            StatusCode::OK => {
                let body: EsGetResponse<EsJournalSource> = resp
                    .json()
                    .await
                    .map_err(|_| net_error("parse journal response"))?;
                Ok(body.source)
            }
            StatusCode::NOT_FOUND => Ok(None),
            s => Err(http_error(s, "get journal")),
        }
    }
}

#[async_trait]
impl Store for ElasticsearchStore {
    async fn load(
        &self,
        name: &str,
        pagination: &Pagination,
    ) -> Result<(JournalFile, usize), StoreError> {
        let journal_source = self
            .fetch_journal(name)
            .await?
            .ok_or_else(|| StoreError::NotFound(name.into()))?;

        const MAX_RESULT_WINDOW: usize = 10_000;

        let from = pagination.offset.unwrap_or(0).min(MAX_RESULT_WINDOW);
        let remaining = MAX_RESULT_WINDOW.saturating_sub(from);
        // Cap at ES's default max_result_window so from+size never exceeds it.
        let size = pagination.limit.unwrap_or(MAX_RESULT_WINDOW).min(remaining);

        let query = json!({
            "query": {
                "bool": {
                    "filter": [
                        { "term": { "event.dataset": "foray.item" } },
                        { "term": { "journal_name": name } }
                    ]
                }
            },
            // _shard_doc is the recommended stable tie-breaker for from/size pagination.
            // See: https://www.elastic.co/docs/reference/elasticsearch/rest-apis/paginate-search-results
            "sort": [{ "@timestamp": "asc" }, { "_shard_doc": "asc" }],
            "from": from,
            "size": size,
            "track_total_hits": true
        });

        let resp = self
            .request(reqwest::Method::POST, &self.search_url())
            .json(&query)
            .send()
            .await
            .map_err(|_| net_error("search items"))?;

        if resp.status() != StatusCode::OK {
            return Err(http_error(resp.status(), "search items"));
        }

        let search: EsSearchResponse<EsItemSource> = resp
            .json()
            .await
            .map_err(|_| net_error("parse items response"))?;

        let total = search.hits.total.value;
        let items: Vec<JournalItem> = search
            .hits
            .hits
            .into_iter()
            .map(|hit| JournalItem {
                id: hit.id.strip_prefix("item:").unwrap_or(&hit.id).to_string(),
                item_type: hit.source.item_type,
                content: hit.source.message,
                tags: hit.source.tags,
                added_at: hit.source.timestamp,
                meta: hit.source.meta,
            })
            .collect();

        let journal = JournalFile {
            schema: migrate::CURRENT_SCHEMA,
            name: journal_source.entity.name,
            title: journal_source.entity.display_name,
            items,
            meta: journal_source.meta,
        };

        Ok((journal, total))
    }

    async fn create(
        &self,
        name: &str,
        title: Option<String>,
        meta: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), StoreError> {
        let doc = EsJournalDoc {
            event: EsEvent {
                dataset: "foray.journal".into(),
            },
            entity: EsEntity {
                name: name.into(),
                display_name: title,
            },
            archived: false,
            meta,
        };

        let url = format!(
            "{}?op_type=create",
            self.doc_url(&format!("journal:{name}"))
        );
        let resp = self
            .request(reqwest::Method::PUT, &url)
            .json(&doc)
            .send()
            .await
            .map_err(|_| net_error("create journal"))?;

        match resp.status() {
            StatusCode::CREATED => {}
            StatusCode::CONFLICT => return Err(StoreError::AlreadyExists(name.into())),
            s => return Err(http_error(s, "create journal")),
        }

        Ok(())
    }

    async fn add_items(
        &self,
        name: &str,
        items: &[JournalItem],
    ) -> Result<Vec<String>, StoreError> {
        let journal = self
            .fetch_journal(name)
            .await?
            .ok_or_else(|| StoreError::NotFound(name.into()))?;
        if journal.archived {
            return Err(StoreError::Archived(name.into()));
        }

        if items.is_empty() {
            return Ok(vec![]);
        }

        // Build NDJSON bulk body
        let mut body = String::new();
        for item in items {
            let action = json!({ "create": { "_id": format!("item:{}", item.id) } });
            let doc = EsItemDoc {
                timestamp: &item.added_at,
                message: &item.content,
                tags: item.tags.as_ref(),
                event: EsEvent {
                    dataset: "foray.item".into(),
                },
                journal_name: name,
                item_type: &item.item_type,
                meta: item.meta.as_ref(),
            };
            body.push_str(&serde_json::to_string(&action)?);
            body.push('\n');
            body.push_str(&serde_json::to_string(&doc)?);
            body.push('\n');
        }

        let resp = self
            .request(reqwest::Method::POST, &self.bulk_url())
            .header("Content-Type", "application/x-ndjson")
            .body(body)
            .send()
            .await
            .map_err(|_| net_error("bulk index items"))?;

        if resp.status() != StatusCode::OK {
            return Err(http_error(resp.status(), "bulk index items"));
        }
        let bulk: EsBulkResponse = resp
            .json()
            .await
            .map_err(|_| net_error("parse bulk response"))?;

        if !bulk.errors {
            return Ok(vec![]);
        }

        // Inspect per-item results: 200/201 = success, 409 = conflict (return ID to caller),
        // anything else = fatal error.
        if bulk.items.len() != items.len() {
            return Err(StoreError::Io(std::io::Error::other(format!(
                "elasticsearch bulk index: expected {} result(s), got {}",
                items.len(),
                bulk.items.len()
            ))));
        }
        let mut failed_ids: Vec<String> = Vec::new();
        for (item, result) in items.iter().zip(bulk.items.iter()) {
            match result.create.status {
                200 | 201 => {}
                409 => failed_ids.push(item.id.clone()),
                _ => {
                    return Err(StoreError::Io(std::io::Error::other(
                        "elasticsearch bulk index: one or more items failed to index",
                    )));
                }
            }
        }

        Ok(failed_ids)
    }

    async fn list(
        &self,
        pagination: &Pagination,
        archived: bool,
    ) -> Result<(Vec<JournalSummary>, usize), StoreError> {
        const MAX_RESULT_WINDOW: usize = 10_000;

        let from = pagination.offset.unwrap_or(0).min(MAX_RESULT_WINDOW);
        let remaining = MAX_RESULT_WINDOW.saturating_sub(from);
        let size = pagination.limit.unwrap_or(1_000).min(remaining);

        let query = json!({
            "query": {
                "bool": {
                    "filter": [
                        { "term": { "event.dataset": "foray.journal" } },
                        { "term": { "archived": archived } }
                    ]
                }
            },
            "sort": [{ "entity.name": "asc" }],
            "from": from,
            "size": size,
            "track_total_hits": true
        });

        let resp = self
            .request(reqwest::Method::POST, &self.search_url())
            .json(&query)
            .send()
            .await
            .map_err(|_| net_error("list journals"))?;

        if resp.status() == StatusCode::NOT_FOUND {
            return Ok((vec![], 0));
        }
        if resp.status() != StatusCode::OK {
            return Err(http_error(resp.status(), "list journals"));
        }

        let search: EsSearchResponse<EsJournalSource> = resp
            .json()
            .await
            .map_err(|_| net_error("parse journals response"))?;

        let total = search.hits.total.value;
        if search.hits.hits.is_empty() {
            return Ok((vec![], total));
        }

        // Fetch item counts for all journals via a terms aggregation (two requests total,
        // not N+1). Result is approximate for large cardinality but acceptable as a hint.
        let agg_query = json!({
            "size": 0,
            "query": { "term": { "event.dataset": "foray.item" } },
            "aggs": {
                "counts": {
                    "terms": { "field": "journal_name", "size": 10_000 }
                }
            }
        });

        let counts: HashMap<String, usize> = {
            let agg_resp = self
                .request(reqwest::Method::POST, &self.search_url())
                .json(&agg_query)
                .send()
                .await
                .map_err(|_| net_error("item count aggregation"))?;

            if agg_resp.status() == StatusCode::OK {
                let agg: serde_json::Value = agg_resp
                    .json()
                    .await
                    .map_err(|_| net_error("parse agg response"))?;
                agg["aggregations"]["counts"]["buckets"]
                    .as_array()
                    .map(|buckets| {
                        buckets
                            .iter()
                            .filter_map(|b| {
                                let key = b["key"].as_str()?.to_string();
                                let count = b["doc_count"].as_u64()? as usize;
                                Some((key, count))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                return Err(net_error("item count aggregation"));
            }
        };

        let summaries = search
            .hits
            .hits
            .into_iter()
            .map(|hit| {
                let name = hit.source.entity.name;
                let count = counts.get(&name).copied().unwrap_or(0);
                JournalSummary {
                    name,
                    title: hit.source.entity.display_name,
                    item_count: count,
                    meta: hit.source.meta,
                }
            })
            .collect();

        Ok((summaries, total))
    }

    async fn delete(&self, name: &str) -> Result<(), StoreError> {
        if self.fetch_journal(name).await?.is_none() {
            return Err(StoreError::NotFound(name.into()));
        }

        // Delete items first — if this fails the journal doc is still present
        // and callers can safely retry.
        let query = json!({
            "query": {
                "bool": {
                    "filter": [
                        { "term": { "event.dataset": "foray.item" } },
                        { "term": { "journal_name": name } }
                    ]
                }
            }
        });
        let resp = self
            .request(reqwest::Method::POST, &self.delete_by_query_url())
            .json(&query)
            .send()
            .await
            .map_err(|_| net_error("delete journal items"))?;

        if resp.status() != StatusCode::OK {
            return Err(http_error(resp.status(), "delete journal items"));
        }

        let url = self.doc_url(&format!("journal:{name}"));
        let resp = self
            .request(reqwest::Method::DELETE, &url)
            .send()
            .await
            .map_err(|_| net_error("delete journal"))?;

        match resp.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(StoreError::NotFound(name.into())),
            s => return Err(http_error(s, "delete journal")),
        }

        Ok(())
    }

    async fn exists(&self, name: &str) -> Result<bool, StoreError> {
        let url = self.doc_url(&format!("journal:{name}"));
        let resp = self
            .request(reqwest::Method::HEAD, &url)
            .send()
            .await
            .map_err(|_| net_error("exists check"))?;

        match resp.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            s => Err(http_error(s, "exists check")),
        }
    }

    async fn archive(&self, name: &str) -> Result<(), StoreError> {
        let journal = self
            .fetch_journal(name)
            .await?
            .ok_or_else(|| StoreError::NotFound(name.into()))?;

        if journal.archived {
            return Err(StoreError::Archived(name.into()));
        }

        let url = self.update_url(&format!("journal:{name}"));
        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&json!({ "doc": { "archived": true } }))
            .send()
            .await
            .map_err(|_| net_error("archive journal"))?;

        match resp.status() {
            StatusCode::OK => Ok(()),
            StatusCode::NOT_FOUND => Err(StoreError::NotFound(name.into())),
            s => Err(http_error(s, "archive journal")),
        }
    }

    async fn unarchive(&self, name: &str) -> Result<(), StoreError> {
        let journal = self
            .fetch_journal(name)
            .await?
            .ok_or_else(|| StoreError::NotFound(name.into()))?;

        if !journal.archived {
            return Ok(());
        }

        let url = self.update_url(&format!("journal:{name}"));
        let resp = self
            .request(reqwest::Method::POST, &url)
            .json(&json!({ "doc": { "archived": false } }))
            .send()
            .await
            .map_err(|_| net_error("unarchive journal"))?;

        match resp.status() {
            StatusCode::OK => Ok(()),
            StatusCode::NOT_FOUND => Err(StoreError::NotFound(name.into())),
            s => Err(http_error(s, "unarchive journal")),
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Percent-encode a document ID for use in ES URL paths.
/// Journal names are `[a-z0-9_-]`; item IDs use consonants and dashes.
/// The only character requiring encoding is the `:` prefix separator.
fn encode_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len() + 4);
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn net_error(op: &str) -> StoreError {
    StoreError::Io(std::io::Error::other(format!(
        "elasticsearch {op}: network error"
    )))
}

fn http_error(status: StatusCode, op: &str) -> StoreError {
    StoreError::Io(std::io::Error::other(format!(
        "elasticsearch {op}: unexpected status {}",
        status.as_u16()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_id_passthrough() {
        assert_eq!(encode_id("journal:my-journal"), "journal%3Amy-journal");
        assert_eq!(encode_id("item:abc-def"), "item%3Aabc-def");
    }

    #[test]
    fn encode_id_no_encoding_needed() {
        assert_eq!(encode_id("plain"), "plain");
        assert_eq!(
            encode_id("with-dash_and_underscore"),
            "with-dash_and_underscore"
        );
    }

    #[test]
    fn auth_username_without_password_errors() {
        let err = ElasticsearchStore::new(
            "http://localhost:9200/idx".into(),
            None,
            Some("user".into()),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("together"), "error was: {err}");
    }

    #[test]
    fn auth_password_without_username_errors() {
        let err = ElasticsearchStore::new(
            "http://localhost:9200/idx".into(),
            None,
            None,
            Some("pass".into()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("together"), "error was: {err}");
    }

    #[test]
    fn url_with_embedded_credentials_rejected() {
        let err = ElasticsearchStore::new(
            "https://admin:secret@es.example.com/foray".into(),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("embed credentials"),
            "error was: {err}"
        );
    }

    #[test]
    fn url_with_username_only_rejected() {
        let err = ElasticsearchStore::new(
            "https://admin@es.example.com/foray".into(),
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("embed credentials"),
            "error was: {err}"
        );
    }

    #[test]
    fn invalid_url_rejected() {
        let err = ElasticsearchStore::new("not a url at all".into(), None, None, None).unwrap_err();
        assert!(err.to_string().contains("invalid URL"), "error was: {err}");
    }

    #[test]
    fn auth_api_key_and_basic_mutually_exclusive() {
        let err = ElasticsearchStore::new(
            "http://localhost:9200/idx".into(),
            Some("key".into()),
            Some("user".into()),
            Some("pass".into()),
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("mutually exclusive"),
            "error was: {err}"
        );
    }

    #[test]
    fn url_with_query_string_rejected() {
        let err =
            ElasticsearchStore::new("http://localhost:9200/idx?pretty".into(), None, None, None)
                .unwrap_err();
        assert!(err.to_string().contains("query string"), "error was: {err}");
    }

    #[test]
    fn url_with_fragment_rejected() {
        let err = ElasticsearchStore::new("http://localhost:9200/idx#top".into(), None, None, None)
            .unwrap_err();
        assert!(err.to_string().contains("fragment"), "error was: {err}");
    }

    #[test]
    fn url_without_index_segment_rejected() {
        for bad in &["http://localhost:9200", "http://localhost:9200/"] {
            let err = ElasticsearchStore::new((*bad).into(), None, None, None).unwrap_err();
            assert!(
                err.to_string().contains("index name"),
                "url={bad} error was: {err}"
            );
        }
    }

    // ── Error scrubbing ──────────────────────────────────────────────

    #[test]
    fn http_error_exposes_only_status_and_op() {
        let err = http_error(StatusCode::INTERNAL_SERVER_ERROR, "create journal");
        let msg = err.to_string();
        assert!(msg.contains("500"), "should contain status code: {msg}");
        assert!(msg.contains("create journal"), "should contain op: {msg}");
        // Must not contain any ES-specific detail (stack trace, index name, shard info).
        assert!(!msg.contains("index"), "must not leak index info: {msg}");
    }

    #[test]
    fn net_error_exposes_only_op() {
        let err = net_error("bulk index items");
        let msg = err.to_string();
        assert!(msg.contains("bulk index items"), "should contain op: {msg}");
        assert!(msg.contains("network error"), "generic message: {msg}");
    }

    // ── encode_id vs path traversal ──────────────────────────────────

    #[test]
    fn encode_id_neutralises_path_traversal() {
        // Dots and slashes must be percent-encoded so they cannot escape
        // the _doc/{id} URL segment.
        let encoded = encode_id("journal:../../_cluster/settings");
        assert!(!encoded.contains('/'), "slashes must be encoded: {encoded}");
        assert!(
            !encoded.contains(".."),
            "dot-dot must be encoded: {encoded}"
        );
    }

    // ── Server-stamped fields ────────────────────────────────────────

    #[test]
    fn item_doc_stamps_journal_name_and_dataset() {
        // Verify that EsItemDoc takes journal_name from the store method's
        // `name` parameter (not from the item) and hardcodes event.dataset.
        let ts = Utc::now();
        let doc = EsItemDoc {
            timestamp: &ts,
            message: "test",
            tags: None,
            event: EsEvent {
                dataset: "foray.item".into(),
            },
            journal_name: "server-provided",
            item_type: &ItemType::Note,
            meta: None,
        };
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["journal_name"], "server-provided");
        assert_eq!(json["event"]["dataset"], "foray.item");
    }
}
