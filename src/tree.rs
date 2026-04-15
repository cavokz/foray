use crate::types::ContextSummary;

/// Build an ASCII tree showing fork lineage from a list of context summaries.
pub fn build_tree(summaries: &[ContextSummary]) -> String {
    if summaries.is_empty() {
        return String::from("(no contexts)");
    }

    let mut lines = Vec::new();

    // Find roots (no parent) and children
    let roots: Vec<&ContextSummary> = summaries.iter().filter(|s| s.parent.is_none()).collect();

    // Orphans: contexts whose parent doesn't exist in the list
    let orphans: Vec<&ContextSummary> = summaries
        .iter()
        .filter(|s| {
            if let Some(ref parent) = s.parent {
                !summaries.iter().any(|other| other.name == *parent)
            } else {
                false
            }
        })
        .collect();

    for root in &roots {
        render_node(&mut lines, summaries, root, "", true);
    }

    for orphan in &orphans {
        render_node(&mut lines, summaries, orphan, "", true);
    }

    lines.join("\n")
}

fn render_node(
    lines: &mut Vec<String>,
    all: &[ContextSummary],
    node: &ContextSummary,
    prefix: &str,
    _is_last: bool,
) {
    let marker = if node.active { "* " } else { "  " };
    let parent_info = node
        .parent
        .as_ref()
        .map(|p| format!(" (forked from {})", p))
        .unwrap_or_default();

    lines.push(format!(
        "{}{}{} [{} items]{}",
        prefix, marker, node.name, node.item_count, parent_info
    ));

    let children: Vec<&ContextSummary> = all
        .iter()
        .filter(|s| s.parent.as_deref() == Some(&node.name))
        .collect();

    for (i, child) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        let child_marker = if child.active { "* " } else { "  " };
        let child_parent_info = child
            .parent
            .as_ref()
            .map(|p| format!(" (forked from {})", p))
            .unwrap_or_default();

        lines.push(format!(
            "{}{}{}{} [{} items]{}",
            prefix, connector, child_marker, child.name, child.item_count, child_parent_info
        ));

        // Recurse for grandchildren
        let grandchildren: Vec<&ContextSummary> = all
            .iter()
            .filter(|s| s.parent.as_deref() == Some(&child.name))
            .collect();

        for (j, grandchild) in grandchildren.iter().enumerate() {
            let gc_is_last = j == grandchildren.len() - 1;
            let gc_prefix = format!("{}{}", prefix, child_prefix);
            render_node(lines, all, grandchild, &gc_prefix, gc_is_last);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(name: &str, items: usize, active: bool, parent: Option<&str>) -> ContextSummary {
        ContextSummary {
            name: name.to_string(),
            item_count: items,
            active,
            parent: parent.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_empty() {
        assert_eq!(build_tree(&[]), "(no contexts)");
    }

    #[test]
    fn test_single_root() {
        let s = vec![summary("main", 3, true, None)];
        let tree = build_tree(&s);
        assert!(tree.contains("* main"));
        assert!(tree.contains("[3 items]"));
    }

    #[test]
    fn test_fork_tree() {
        let s = vec![
            summary("auth-triage", 2, false, None),
            summary("auth-deep-dive", 3, true, Some("auth-triage")),
        ];
        let tree = build_tree(&s);
        assert!(tree.contains("auth-triage"));
        assert!(tree.contains("auth-deep-dive"));
        assert!(tree.contains("forked from auth-triage"));
    }

    #[test]
    fn test_multiple_roots() {
        let s = vec![
            summary("alpha", 1, false, None),
            summary("beta", 2, true, None),
        ];
        let tree = build_tree(&s);
        assert!(tree.contains("alpha"));
        assert!(tree.contains("* beta"));
    }
}
