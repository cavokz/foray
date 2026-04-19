//! Integration test: StdioStore → foray serve subprocess over MCP stdio.

use chrono::Utc;
use foray::store::Store;
use foray::store_stdio::StdioStore;
use foray::types::{ItemType, JournalFile, JournalItem, Pagination, item_id};

/// Spawn `foray serve` in an isolated home directory, then exercise
/// `create`, `load`, and `list` through a `StdioStore`.
#[tokio::test]
async fn stdio_store_create_load_list() {
    // Isolated home dir so the subprocess doesn't touch ~.
    let home = tempfile::TempDir::new().unwrap();

    let home_str = home.path().to_str().unwrap().to_string();
    #[allow(unused_mut)]
    let mut env_overrides = vec![("HOME".to_string(), home_str.clone())];
    // On Windows, dirs::home_dir() ignores HOME and uses USERPROFILE /
    // HOMEDRIVE+HOMEPATH instead.  Override all three so the subprocess
    // is truly isolated regardless of platform.
    #[cfg(windows)]
    {
        env_overrides.push(("USERPROFILE".to_string(), home_str.clone()));
        env_overrides.push(("HOMEDRIVE".to_string(), "".to_string()));
        env_overrides.push(("HOMEPATH".to_string(), home_str.clone()));
    }

    let store = StdioStore::new(
        env!("CARGO_BIN_EXE_foray").to_string(),
        vec![],
        env_overrides,
        None, // use first store from hello
    );

    // ── create ────────────────────────────────────────────────────────
    let journal = JournalFile::new("remote-test", Some("Remote Test Journal".into()), None);
    store.create(journal).await.expect("create should succeed");

    // Creating the same journal again must error with AlreadyExists.
    let dup = JournalFile::new("remote-test", Some("Dup".into()), None);
    let err = store.create(dup).await.unwrap_err();
    assert!(
        matches!(err, foray::store::StoreError::AlreadyExists(_)),
        "expected AlreadyExists, got {err:?}"
    );

    // ── add_items ────────────────────────────────────────────────────
    let item = JournalItem {
        id: item_id(),
        item_type: ItemType::Finding,
        content: "hello from remote".to_string(),
        file_ref: None,
        tags: None,
        added_at: Utc::now(),
        meta: None,
    };
    let total = store
        .add_items("remote-test", vec![item])
        .await
        .expect("add_items should succeed");
    assert_eq!(total, 1, "journal should now have 1 item");

    // ── load ─────────────────────────────────────────────────────────
    let (loaded, item_total) = store
        .load("remote-test", &Pagination::default())
        .await
        .expect("load should succeed");

    assert_eq!(loaded.name, "remote-test");
    assert_eq!(loaded.title.as_deref(), Some("Remote Test Journal"));
    assert_eq!(item_total, 1);
    assert_eq!(loaded.items.len(), 1);
    assert_eq!(loaded.items[0].content, "hello from remote");

    // ── exists ────────────────────────────────────────────────────────
    assert!(store.exists("remote-test").await.unwrap());
    assert!(!store.exists("no-such-journal").await.unwrap());

    // ── load not found ───────────────────────────────────────────────
    let not_found = store.load("no-such-journal", &Pagination::default()).await;
    assert!(
        matches!(not_found, Err(foray::store::StoreError::NotFound(_))),
        "expected NotFound, got {not_found:?}"
    );

    // ── list ─────────────────────────────────────────────────────────
    let (summaries, list_total) = store
        .list(&Pagination::default(), false)
        .await
        .expect("list should succeed");

    assert_eq!(list_total, 1);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "remote-test");
    assert_eq!(summaries[0].item_count, 1);
}
