//! Online integration tests for ElasticsearchStore.
//!
//! All tests are marked `#[ignore]` and require a live Elasticsearch instance.
//! Each test creates and tears down its own uniquely-named index so tests can
//! run in parallel safely.
//!
//! Usage:
//!   ES_TEST_URL=http://localhost:9292 \
//!   ES_TEST_USER=elastic \
//!   ES_TEST_PASSWORD=changeme \
//!   cargo test --test elasticsearch_store_test -- --include-ignored

use chrono::Utc;
use foray::store::{Store, StoreError};
use foray::store_elasticsearch::ElasticsearchStore;
use foray::types::{ItemType, JournalItem, Pagination, item_id};
use rand::Rng;

// ── Test harness ─────────────────────────────────────────────────────

fn es_env() -> (String, String, String) {
    let url =
        std::env::var("ES_TEST_URL").expect("ES_TEST_URL must be set to run ES integration tests");
    let user = std::env::var("ES_TEST_USER").unwrap_or_else(|_| "elastic".into());
    let pass = std::env::var("ES_TEST_PASSWORD").unwrap_or_else(|_| "changeme".into());
    (url, user, pass)
}

struct TestIndex {
    pub store: ElasticsearchStore,
    base_url: String,
    index: String,
    user: String,
    pass: String,
}

impl TestIndex {
    fn new() -> Self {
        let (base_url, user, pass) = es_env();
        let suffix: u32 = rand::rng().random();
        let index = format!("foray-test-{suffix:08x}");
        let index_url = format!("{base_url}/{index}");
        let store =
            ElasticsearchStore::new(index_url, None, Some(user.clone()), Some(pass.clone()))
                .expect("ElasticsearchStore::new");
        Self {
            store,
            base_url,
            index,
            user,
            pass,
        }
    }

    /// Force all pending writes visible. Call after every mutation.
    async fn refresh(&self) {
        reqwest::Client::new()
            .post(format!("{}/{}/_refresh", self.base_url, self.index))
            .basic_auth(&self.user, Some(&self.pass))
            .send()
            .await
            .expect("refresh")
            .error_for_status()
            .expect("refresh returned non-success status");
    }

    /// Delete the test index.
    async fn cleanup(&self) {
        let _ = reqwest::Client::new()
            .delete(format!("{}/{}", self.base_url, self.index))
            .basic_auth(&self.user, Some(&self.pass))
            .send()
            .await;
    }
}

impl Drop for TestIndex {
    fn drop(&mut self) {
        // Best-effort cleanup: delete the test index even if a test panics.
        // Use try_current() so we don't panic if no runtime is active during unwinding.
        let url = format!("{}/{}", self.base_url, self.index);
        let user = self.user.clone();
        let pass = self.pass.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = reqwest::Client::new()
                    .delete(&url)
                    .basic_auth(&user, Some(&pass))
                    .send()
                    .await;
            });
        }
    }
}

fn item(content: &str) -> JournalItem {
    JournalItem {
        id: item_id(),
        item_type: ItemType::Note,
        content: content.to_string(),
        tags: None,
        added_at: Utc::now(),
        meta: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_create_and_load() {
    let t = TestIndex::new();

    t.store
        .create("my-journal", Some("My Journal".into()), None)
        .await
        .expect("create");
    t.refresh().await;

    let (loaded, count) = t
        .store
        .load("my-journal", &Pagination::default())
        .await
        .expect("load");
    assert_eq!(loaded.name, "my-journal");
    assert_eq!(loaded.title.as_deref(), Some("My Journal"));
    assert_eq!(count, 0);
    assert!(loaded.items.is_empty());

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_create_duplicate_errors() {
    let t = TestIndex::new();

    t.store
        .create("dup", None, None)
        .await
        .expect("first create");

    let err = t.store.create("dup", None, None).await.unwrap_err();
    assert!(
        matches!(err, StoreError::AlreadyExists(_)),
        "expected AlreadyExists, got {err:?}"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_load_not_found() {
    let t = TestIndex::new();

    let err = t
        .store
        .load("no-such", &Pagination::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_add_items_and_load() {
    let t = TestIndex::new();

    t.store
        .create("items-test", None, None)
        .await
        .expect("create");
    t.refresh().await;

    let failed = t
        .store
        .add_items(
            "items-test",
            &[item("first"), item("second"), item("third")],
        )
        .await
        .expect("add_items");
    assert!(failed.is_empty(), "no items should fail: {failed:?}");
    t.refresh().await;

    let (loaded, total) = t
        .store
        .load("items-test", &Pagination::default())
        .await
        .expect("load");
    assert_eq!(total, 3);
    assert_eq!(loaded.items.len(), 3);
    let contents: Vec<&str> = loaded.items.iter().map(|i| i.content.as_str()).collect();
    assert!(contents.contains(&"first"));
    assert!(contents.contains(&"second"));
    assert!(contents.contains(&"third"));

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_exists() {
    let t = TestIndex::new();

    assert!(
        !t.store.exists("no-such").await.expect("exists false"),
        "should not exist"
    );

    t.store
        .create("existing", None, None)
        .await
        .expect("create");
    t.refresh().await;

    assert!(
        t.store.exists("existing").await.expect("exists true"),
        "should exist"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_list() {
    let t = TestIndex::new();

    for name in ["alpha", "beta", "gamma"] {
        t.store.create(name, None, None).await.expect("create");
    }
    // &[...] temporary lives to end-of-statement, safely covering .await.
    assert!(
        t.store
            .add_items("beta", &[item("x"), item("y")])
            .await
            .expect("add_items")
            .is_empty(),
        "no items should fail"
    );
    t.refresh().await;

    let (summaries, total) = t
        .store
        .list(&Pagination::default(), false)
        .await
        .expect("list");
    assert_eq!(total, 3);
    assert_eq!(summaries.len(), 3);

    let beta = summaries.iter().find(|s| s.name == "beta").expect("beta");
    assert_eq!(beta.item_count, 2);

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_list_excludes_archived() {
    let t = TestIndex::new();

    for name in ["active", "archived-one"] {
        t.store.create(name, None, None).await.expect("create");
    }
    t.store.archive("archived-one").await.expect("archive");
    t.refresh().await;

    let (active, active_total) = t
        .store
        .list(&Pagination::default(), false)
        .await
        .expect("list active");
    assert_eq!(active_total, 1);
    assert_eq!(active[0].name, "active");

    let (archived, archived_total) = t
        .store
        .list(&Pagination::default(), true)
        .await
        .expect("list archived");
    assert_eq!(archived_total, 1);
    assert_eq!(archived[0].name, "archived-one");

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_delete() {
    let t = TestIndex::new();

    t.store
        .create("to-delete", None, None)
        .await
        .expect("create");
    // &[...] temporary lives to end-of-statement, safely covering .await.
    assert!(
        t.store
            .add_items("to-delete", &[item("item1"), item("item2")])
            .await
            .expect("add_items")
            .is_empty(),
        "no items should fail"
    );
    t.refresh().await;

    t.store.delete("to-delete").await.expect("delete");
    t.refresh().await;

    let err = t
        .store
        .load("to-delete", &Pagination::default())
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound(_)),
        "expected NotFound after delete, got {err:?}"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_delete_not_found() {
    let t = TestIndex::new();

    let err = t.store.delete("no-such").await.unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_archive_and_unarchive() {
    let t = TestIndex::new();

    t.store
        .create("archivable", None, None)
        .await
        .expect("create");
    t.refresh().await;

    // Archive it — should disappear from active list.
    t.store.archive("archivable").await.expect("archive");
    t.refresh().await;

    let (active, _) = t
        .store
        .list(&Pagination::default(), false)
        .await
        .expect("list active");
    assert!(
        !active.iter().any(|s| s.name == "archivable"),
        "archived journal should not appear in active list"
    );
    let (archived, _) = t
        .store
        .list(&Pagination::default(), true)
        .await
        .expect("list archived");
    assert!(
        archived.iter().any(|s| s.name == "archivable"),
        "archived journal should appear in archived list"
    );

    // Unarchive — should come back to active list.
    t.store.unarchive("archivable").await.expect("unarchive");
    t.refresh().await;

    let (active, _) = t
        .store
        .list(&Pagination::default(), false)
        .await
        .expect("list active after unarchive");
    assert!(
        active.iter().any(|s| s.name == "archivable"),
        "unarchived journal should appear in active list"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_archive_not_found() {
    let t = TestIndex::new();

    let err = t.store.archive("no-such").await.unwrap_err();
    assert!(
        matches!(err, StoreError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_create_with_items() {
    let t = TestIndex::new();

    t.store
        .create("with-items", Some("Has Items".into()), None)
        .await
        .expect("create");
    // &[...] temporary lives to end-of-statement, safely covering .await.
    assert!(
        t.store
            .add_items("with-items", &[item("alpha"), item("beta")])
            .await
            .expect("add_items")
            .is_empty(),
        "no items should fail"
    );
    t.refresh().await;

    let (loaded, total) = t
        .store
        .load("with-items", &Pagination::default())
        .await
        .expect("load");
    assert_eq!(loaded.title.as_deref(), Some("Has Items"));
    assert_eq!(total, 2);
    assert_eq!(loaded.items.len(), 2);
    let contents: Vec<&str> = loaded.items.iter().map(|i| i.content.as_str()).collect();
    assert!(contents.contains(&"alpha"));
    assert!(contents.contains(&"beta"));

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_add_items_to_archived_errors() {
    let t = TestIndex::new();

    t.store.create("frozen", None, None).await.expect("create");
    t.refresh().await;
    t.store.archive("frozen").await.expect("archive");
    t.refresh().await;

    // &[...] temporary lives to end-of-statement, safely covering .await.
    let err = t
        .store
        .add_items("frozen", &[item("nope")])
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Archived(_)),
        "expected Archived, got {err:?}"
    );

    t.cleanup().await;
}

#[tokio::test]
#[ignore = "requires ES_TEST_URL"]
async fn es_pagination() {
    let t = TestIndex::new();

    t.store.create("paged", None, None).await.expect("create");
    let items = (0..5)
        .map(|i| item(&format!("item-{i}")))
        .collect::<Vec<_>>();
    assert!(
        t.store
            .add_items("paged", &items)
            .await
            .expect("add_items")
            .is_empty(),
        "no items should fail"
    );
    t.refresh().await;

    let page = Pagination {
        offset: Some(0),
        limit: Some(2),
    };
    let (loaded, total) = t.store.load("paged", &page).await.expect("load page 1");
    assert_eq!(total, 5);
    assert_eq!(loaded.items.len(), 2);

    t.cleanup().await;
}
