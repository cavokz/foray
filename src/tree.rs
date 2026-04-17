use crate::types::{ItemType, JournalSummary};

/// Info extracted from a fork item.
pub struct ForkInfo {
    pub parent: String,
}

/// Parse fork info from a `foray:<name>#<id>` ref string.
pub fn parse_fork_ref(ref_str: &str) -> Option<ForkInfo> {
    let rest = ref_str.strip_prefix("foray:")?;
    let parent = rest.split('#').next()?.to_string();
    if parent.is_empty() {
        return None;
    }
    Some(ForkInfo { parent })
}

/// Extract fork infos from journal items.
pub fn extract_fork_infos(items: &[crate::types::JournalItem]) -> Vec<ForkInfo> {
    items
        .iter()
        .filter(|item| item.item_type == ItemType::Fork)
        .filter_map(|item| item.file_ref.as_deref().and_then(parse_fork_ref))
        .collect()
}

/// Build an ASCII fork-lineage tree from journal summaries and fork data.
pub fn build_tree(
    summaries: &[JournalSummary],
    journals_items: &[(String, Vec<ForkInfo>)],
) -> String {
    let mut children: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut has_parent: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (name, fork_infos) in journals_items {
        for info in fork_infos {
            children
                .entry(info.parent.clone())
                .or_default()
                .push(name.clone());
            has_parent.insert(name.clone());
        }
    }

    let mut roots: Vec<&str> = summaries
        .iter()
        .filter(|s| !has_parent.contains(&s.name))
        .map(|s| s.name.as_str())
        .collect();
    roots.sort();

    let mut output = String::new();
    let mut visited = std::collections::HashSet::new();
    for root in &roots {
        render_node(&mut output, root, &children, "", true, true, &mut visited);
    }
    output
}

fn render_node(
    output: &mut String,
    name: &str,
    children: &std::collections::HashMap<String, Vec<String>>,
    prefix: &str,
    is_root: bool,
    is_last: bool,
    visited: &mut std::collections::HashSet<String>,
) {
    if !visited.insert(name.to_string()) {
        let connector = if is_root {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };
        output.push_str(&format!("{prefix}{connector}{name} (cycle)\n"));
        return;
    }

    if is_root {
        output.push_str(&format!("{name}\n"));
    } else {
        let connector = if is_last { "└── " } else { "├── " };
        output.push_str(&format!("{prefix}{connector}{name}\n"));
    }

    if let Some(kids) = children.get(name) {
        let mut sorted = kids.clone();
        sorted.sort();
        let child_prefix = if is_root {
            String::new()
        } else if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };
        for (i, child) in sorted.iter().enumerate() {
            let child_is_last = i == sorted.len() - 1;
            render_node(
                output,
                child,
                children,
                &child_prefix,
                false,
                child_is_last,
                visited,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::JournalSummary;

    fn summary(name: &str) -> JournalSummary {
        JournalSummary {
            name: name.into(),
            title: None,
            item_count: 0,
            meta: None,
        }
    }

    #[test]
    fn test_empty() {
        assert_eq!(build_tree(&[], &[]), "");
    }

    #[test]
    fn test_single_root() {
        assert_eq!(build_tree(&[summary("root")], &[]), "root\n");
    }

    #[test]
    fn test_fork_tree() {
        let summaries = vec![summary("parent"), summary("child-a"), summary("child-b")];
        let items = vec![
            (
                "child-a".into(),
                vec![ForkInfo {
                    parent: "parent".into(),
                }],
            ),
            (
                "child-b".into(),
                vec![ForkInfo {
                    parent: "parent".into(),
                }],
            ),
        ];
        assert_eq!(
            build_tree(&summaries, &items),
            "parent\n├── child-a\n└── child-b\n"
        );
    }

    #[test]
    fn test_multiple_roots() {
        let summaries = vec![summary("alpha"), summary("beta")];
        assert_eq!(build_tree(&summaries, &[]), "alpha\nbeta\n");
    }

    #[test]
    fn test_parse_fork_ref() {
        let info = parse_fork_ref("foray:auth-triage#abc123").unwrap();
        assert_eq!(info.parent, "auth-triage");
        assert!(parse_fork_ref("not-a-ref").is_none());
        assert!(parse_fork_ref("foray:#abc").is_none());
    }
}
