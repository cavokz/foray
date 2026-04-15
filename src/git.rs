use std::path::Path;
use std::process::Command;

/// Detect project name from git repo name, falling back to directory name.
pub fn detect_project(workspace: &Path) -> String {
    // Try git remote origin URL first
    if let Some(name) = git_repo_name(workspace) {
        return sanitize_project_name(&name);
    }
    // Try git toplevel directory name
    if let Some(name) = git_toplevel_name(workspace) {
        return sanitize_project_name(&name);
    }
    // Fallback to directory name
    workspace
        .file_name()
        .map(|n| sanitize_project_name(&n.to_string_lossy()))
        .unwrap_or_else(|| "default".to_string())
}

/// Detect current git branch.
pub fn detect_branch(workspace: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

fn git_repo_name(workspace: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(workspace)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_repo_name(&url)
}

fn git_toplevel_name(workspace: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(workspace)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let toplevel = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Path::new(&toplevel)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

fn parse_repo_name(url: &str) -> Option<String> {
    // Handle SSH: git@github.com:org/repo.git
    // Handle HTTPS: https://github.com/org/repo.git
    let name = url.rsplit('/').next().or_else(|| url.rsplit(':').next())?;

    let name = name.strip_suffix(".git").unwrap_or(name);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn sanitize_project_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    // Trim leading/trailing hyphens and collapse repeated hyphens
    let mut result = String::new();
    let mut last_was_hyphen = true; // treat start as hyphen to skip leading
    for c in sanitized.chars() {
        if c == '-' {
            if !last_was_hyphen {
                result.push(c);
            }
            last_was_hyphen = true;
        } else {
            result.push(c);
            last_was_hyphen = false;
        }
    }
    // Trim trailing hyphen
    if result.ends_with('-') {
        result.pop();
    }

    if result.is_empty() {
        "default".to_string()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo_name() {
        assert_eq!(
            parse_repo_name("git@github.com:org/my-repo.git"),
            Some("my-repo".to_string())
        );
        assert_eq!(
            parse_repo_name("https://github.com/org/my-repo.git"),
            Some("my-repo".to_string())
        );
        assert_eq!(
            parse_repo_name("https://github.com/org/my-repo"),
            Some("my-repo".to_string())
        );
    }

    #[test]
    fn test_sanitize_project_name() {
        assert_eq!(sanitize_project_name("My Project"), "my-project");
        assert_eq!(sanitize_project_name("hello_world"), "hello_world");
        assert_eq!(sanitize_project_name("ABC--DEF"), "abc-def");
    }
}
