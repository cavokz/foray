//! Integration tests for `foray list --completion`.

use std::process::Command;

fn foray_bin() -> &'static str {
    env!("CARGO_BIN_EXE_foray")
}

fn run_foray(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    let home_str = home.to_str().unwrap();
    let mut cmd = Command::new(foray_bin());
    cmd.args(args)
        .current_dir(home)
        .env("HOME", home_str)
        .env_remove("FORAY_JOURNAL")
        .env_remove("FORAY_STORE")
        .env_remove("COMPLETE");
    // On Windows, home::home_dir() checks USERPROFILE, not HOME.
    #[cfg(windows)]
    cmd.env("USERPROFILE", home_str);
    cmd.output().expect("failed to run foray")
}

fn create_journal(home: &std::path::Path, name: &str, title: &str) {
    let out = run_foray(home, &["create", name, "--title", title]);
    assert!(
        out.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn archive_journal(home: &std::path::Path, name: &str) {
    let out = run_foray(home, &["archive", name]);
    assert!(
        out.status.success(),
        "archive failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn list_completion_outputs_journal_names() {
    let home = tempfile::TempDir::new().unwrap();
    create_journal(home.path(), "alpha", "Alpha Journal");
    create_journal(home.path(), "beta", "Beta Journal");

    let out = run_foray(home.path(), &["list", "--completion"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let names: Vec<&str> = stdout.lines().collect();
    assert!(names.contains(&"alpha"), "expected alpha in {names:?}");
    assert!(names.contains(&"beta"), "expected beta in {names:?}");
}

#[test]
fn list_completion_excludes_archived() {
    let home = tempfile::TempDir::new().unwrap();
    create_journal(home.path(), "active-one", "Active");
    create_journal(home.path(), "to-archive", "To Archive");
    archive_journal(home.path(), "to-archive");

    let out = run_foray(home.path(), &["list", "--completion"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let names: Vec<&str> = stdout.lines().collect();
    assert!(
        names.contains(&"active-one"),
        "active-one should appear: {names:?}"
    );
    assert!(
        !names.contains(&"to-archive"),
        "archived journal must not appear in active list: {names:?}"
    );
}

#[test]
fn list_completion_archived_flag() {
    let home = tempfile::TempDir::new().unwrap();
    create_journal(home.path(), "keep-active", "Active");
    create_journal(home.path(), "archived-one", "Archived");
    archive_journal(home.path(), "archived-one");

    let out = run_foray(home.path(), &["list", "--completion", "--archived"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let names: Vec<&str> = stdout.lines().collect();
    assert!(
        names.contains(&"archived-one"),
        "expected archived-one: {names:?}"
    );
    assert!(
        !names.contains(&"keep-active"),
        "active journal must not appear in archived list: {names:?}"
    );
}

#[test]
fn list_completion_empty_store() {
    let home = tempfile::TempDir::new().unwrap();
    let out = run_foray(home.path(), &["list", "--completion"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.trim().is_empty(), "expected no output: {stdout:?}");
}
