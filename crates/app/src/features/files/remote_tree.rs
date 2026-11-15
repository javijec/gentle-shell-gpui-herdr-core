//! Lazy remote directory tree for the shared Files dock.
//! All keys are untouched POSIX strings belonging to one selected agent;
//! never turn them into Windows PathBuf values.
use std::collections::{HashMap, HashSet};

use super::backend::WorkspaceDirEntry;

const MAX_VISIBLE_ROWS: usize = 2000;

#[derive(Clone)]
pub(super) struct RemoteTreeRow {
    pub path: String,
    pub entry: WorkspaceDirEntry,
    pub depth: usize,
    pub expanded: bool,
    pub loading: bool,
}

/// Flatten only loaded, expanded directories. The bridge bounds each
/// per-directory response to 500 entries; the UI also has a global cap.
pub(super) fn visible_rows(
    root: &str,
    children: &HashMap<String, Vec<WorkspaceDirEntry>>,
    expanded: &HashSet<String>,
    loading: &HashSet<String>,
) -> Vec<RemoteTreeRow> {
    fn walk(
        directory: &str,
        depth: usize,
        children: &HashMap<String, Vec<WorkspaceDirEntry>>,
        expanded: &HashSet<String>,
        loading: &HashSet<String>,
        rows: &mut Vec<RemoteTreeRow>,
    ) {
        if depth > 32 || rows.len() >= MAX_VISIBLE_ROWS {
            return;
        }
        let Some(entries) = children.get(directory) else { return; };
        for entry in entries {
            if rows.len() >= MAX_VISIBLE_ROWS { break; }
            let path = if directory == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{directory}/{}", entry.name)
            };
            let is_dir = entry.kind == "directory";
            let opened = is_dir && expanded.contains(&path);
            rows.push(RemoteTreeRow {
                path: path.clone(),
                entry: entry.clone(),
                depth,
                expanded: opened,
                loading: loading.contains(&path),
            });
            if opened {
                walk(&path, depth + 1, children, expanded, loading, rows);
            }
        }
    }

    let mut rows = Vec::new();
    walk(root, 0, children, expanded, loading, &mut rows);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: &str) -> WorkspaceDirEntry {
        WorkspaceDirEntry {
            name: name.into(), kind: kind.into(), ignored: false,
        }
    }

    #[test]
    fn remote_tree_is_lazy_and_uses_unmodified_posix_paths() {
        let root = "/srv/workspaces/JaviPi";
        let mut children = HashMap::new();
        children.insert(root.into(), vec![
            entry("src", "directory"), entry("Cargo.toml", "file"),
        ]);
        children.insert(format!("{root}/src"), vec![entry("main.rs", "file")]);
        let collapsed = visible_rows(root, &children, &HashSet::new(), &HashSet::new());
        assert_eq!(collapsed.len(), 2);
        assert_eq!(collapsed[0].path, format!("{root}/src"));
        let mut expanded = HashSet::new();
        expanded.insert(format!("{root}/src"));
        let opened = visible_rows(root, &children, &expanded, &HashSet::new());
        assert_eq!(opened.len(), 3);
        assert_eq!(opened[1].path, format!("{root}/src/main.rs"));
        assert_eq!(opened[1].depth, 1);
    }

    #[test]
    fn filesystem_root_does_not_generate_double_slashes() {
        let mut children = HashMap::new();
        children.insert("/".into(), vec![entry("etc", "directory")]);
        let rows = visible_rows("/", &children, &HashSet::new(), &HashSet::new());
        assert_eq!(rows[0].path, "/etc");
    }
}
