//! Reading and writing `workspaces.json` under the user's data dir.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{SavedPane, SavedState};

fn file_path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("gentle-shell-gpui").join("workspaces.json"))
}

/// The saved list; empty when there is none or it can't be read.
pub fn load() -> SavedState {
    file_path()
        .and_then(|path| load_from(&path))
        .unwrap_or_default()
}

/// The saved list. Panes this build can't restore (a kind from a newer
/// build, say) are dropped, keeping every workspace and tab around them. A
/// file that still doesn't read is copied aside before the list starts
/// empty, since the next save replaces it.
pub(super) fn load_from(path: &Path) -> Option<SavedState> {
    let text = std::fs::read_to_string(path).ok()?;
    if let Ok(state) = serde_json::from_str(&text) {
        return Some(state);
    }
    let lenient = serde_json::from_str::<Value>(&text)
        .ok()
        .map(drop_unknown_panes)
        .and_then(|value| serde_json::from_value(value).ok());
    if lenient.is_none() {
        let _ = std::fs::copy(path, path.with_extension("json.unreadable"));
    }
    lenient
}

/// `state` without the panes that don't read as a [`SavedPane`]: a split
/// keeps its other side, a tab left without panes goes, and the focused
/// pane and active tab fall back to the first.
fn drop_unknown_panes(mut state: Value) -> Value {
    let Some(workspaces) = state.get_mut("workspaces").and_then(Value::as_array_mut) else {
        return state;
    };
    for workspace in workspaces {
        let Some(tabs) = workspace.get_mut("tabs").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = tabs.len();
        tabs.retain_mut(
            |tab| match tab.get_mut("layout").map(Value::take).and_then(prune) {
                Some((layout, changed)) => {
                    tab["layout"] = layout;
                    if changed {
                        tab["focused"] = 0.into();
                    }
                    true
                }
                None => false,
            },
        );
        if tabs.len() != before {
            workspace["active_tab"] = 0.into();
        }
    }
    state
}

/// A layout without its unreadable leaves, and whether any went; `None`
/// when none is left.
fn prune(node: Value) -> Option<(Value, bool)> {
    if let Some(leaf) = node.get("leaf") {
        return serde_json::from_value::<SavedPane>(leaf.clone())
            .is_ok()
            .then_some((node, false));
    }
    let split = node.get("split")?;
    let first = split.get("first").cloned().and_then(prune);
    let second = split.get("second").cloned().and_then(prune);
    match (first, second) {
        (Some((first, a)), Some((second, b))) => {
            let mut node = node;
            node["split"]["first"] = first;
            node["split"]["second"] = second;
            Some((node, a || b))
        }
        (Some((only, _)), None) | (None, Some((only, _))) => Some((only, true)),
        (None, None) => None,
    }
}

pub fn save(state: &SavedState) -> std::io::Result<()> {
    match file_path() {
        Some(path) => save_to(&path, state),
        None => Ok(()),
    }
}

/// Written to a sibling and renamed over, so a crash mid-write keeps the old list.
pub(super) fn save_to(path: &Path, state: &SavedState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}
