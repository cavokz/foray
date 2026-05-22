//! Integration tests for `foray delete` confirmation prompt and `--force` flag.

use std::io::Write;
use std::process::{Command, Stdio};

fn foray_bin() -> &'static str {
    env!("CARGO_BIN_EXE_foray")
}

fn run_foray(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    let home_str = home.to_str().unwrap();
    let mut cmd = Command::new(foray_bin());
    cmd.args(args)
        .current_dir(home)
        .env("FORAY_HOME", home_str)
        .env_remove("HOME")
        .env_remove("FORAY_JOURNAL")
        .env_remove("FORAY_STORE")
        .env_remove("COMPLETE");
    cmd.output().expect("failed to run foray")
}

fn run_foray_with_stdin(
    home: &std::path::Path,
    args: &[&str],
    stdin_input: &str,
) -> std::process::Output {
    let home_str = home.to_str().unwrap();
    let mut cmd = Command::new(foray_bin());
    cmd.args(args)
        .current_dir(home)
        .env("FORAY_HOME", home_str)
        .env_remove("HOME")
        .env_remove("FORAY_JOURNAL")
        .env_remove("FORAY_STORE")
        .env_remove("COMPLETE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn foray");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_input.as_bytes())
        .unwrap();
    child.wait_with_output().expect("failed to wait for foray")
}

fn create_journal(home: &std::path::Path, name: &str) {
    let out = run_foray(home, &["create", name, "--title", "Test Journal"]);
    assert!(
        out.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn journal_exists(home: &std::path::Path, name: &str) -> bool {
    let out = run_foray(home, &["list", "--completion"]);
    assert!(
        out.status.success(),
        "list --completion failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    stdout.lines().any(|l| l == name)
}

#[test]
fn delete_aborts_when_user_declines() {
    let home = tempfile::TempDir::new().unwrap();
    create_journal(home.path(), "my-journal");
    assert!(journal_exists(home.path(), "my-journal"));

    let out = run_foray_with_stdin(home.path(), &["delete", "my-journal"], "n\n");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Aborted."),
        "expected 'Aborted.' in stdout: {stdout:?}"
    );
    assert!(
        journal_exists(home.path(), "my-journal"),
        "journal should still exist after abort"
    );
}

#[test]
fn delete_aborts_on_empty_input() {
    let home = tempfile::TempDir::new().unwrap();
    create_journal(home.path(), "my-journal");

    let out = run_foray_with_stdin(home.path(), &["delete", "my-journal"], "\n");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Aborted."),
        "expected 'Aborted.' in stdout: {stdout:?}"
    );
    assert!(
        journal_exists(home.path(), "my-journal"),
        "journal should still exist after abort"
    );
}

#[test]
fn delete_proceeds_when_user_confirms() {
    let home = tempfile::TempDir::new().unwrap();
    create_journal(home.path(), "my-journal");

    let out = run_foray_with_stdin(home.path(), &["delete", "my-journal"], "y\n");
    assert!(
        out.status.success(),
        "delete failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !journal_exists(home.path(), "my-journal"),
        "journal should be gone after confirmed delete"
    );
}

#[test]
fn delete_force_skips_confirmation() {
    let home = tempfile::TempDir::new().unwrap();
    create_journal(home.path(), "my-journal");

    // No stdin provided — would hang if prompt were shown
    let out = run_foray(home.path(), &["delete", "--force", "my-journal"]);
    assert!(
        out.status.success(),
        "delete --force failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !journal_exists(home.path(), "my-journal"),
        "journal should be gone after --force delete"
    );
}

#[test]
fn delete_fails_before_prompt_when_journal_not_found() {
    let home = tempfile::TempDir::new().unwrap();

    // Run without --force and pipe "y" as stdin: if the existence check were
    // missing, the prompt would appear and the "y" would confirm deletion of a
    // non-existent journal (which would then fail at store.delete). With the
    // check in place, the command fails before ever showing the prompt.
    let out = run_foray_with_stdin(home.path(), &["delete", "no-such-journal"], "y\n");
    assert!(
        !out.status.success(),
        "expected failure for non-existent journal"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not found"),
        "expected 'not found' in stderr: {stderr:?}"
    );
    let combined = format!("{}{}", String::from_utf8_lossy(&out.stdout), stderr);
    assert!(
        !combined.contains("[y/N]"),
        "confirmation prompt must not appear for non-existent journal: {combined:?}"
    );
}
