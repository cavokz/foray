fn main() {
    // Bake git describe output (e.g. "v0.3.0-12-gabcdef-dirty") into the binary.
    // Falls back to "unknown" if git is unavailable or this is not a git repo.
    let describe = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=FORAY_GIT_DESCRIBE={describe}");

    // Re-run if HEAD or index changes.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
