//! Named workspaces (Tern calls them sessions): each has a root folder and its
//! own tabs. The list, with every tab's pane layout, is kept as JSON in
//! `<data dir>/gentle-shell-gpui/workspaces.json` and restored on launch.
//!
//! This module holds the serialized model; `store` reads and writes the file.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::shared::pane::{PaneKind, PaneTree};

mod store;
#[cfg(test)]
mod tests;

pub use gentle_core::workspaces::unique_name;
pub use store::{load, save};

/// User edits made to a remote file before the window closed. The original
/// SHA-256 is retained: a restored draft must never use a newer server
/// revision to overwrite changes made while the desktop was closed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedRemoteDraft {
    pub path: String,
    pub content: String,
    pub revision: String,
}

/// One pane as saved: enough to build it again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPane {
    pub kind: PaneKind,
    pub cwd: PathBuf,
    /// The file a file pane shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// The conversation an agent pane had, resumed when it still exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<PathBuf>,
    /// Persistent Herdr terminal address: local tN or remote machine/tN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr_terminal: Option<String>,
    /// Per-agent machine selection. None preserves legacy 4B environment behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_target: Option<pi_rpc::AgentTarget>,
    /// POSIX file path, serialized separately from a Windows file path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
    /// Exact Herdr terminal of the parent agent, not a machine name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_owner: Option<String>,
    /// Private unsaved text draft. Stored under the local per-user data dir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_draft: Option<SavedRemoteDraft>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub layout: PaneTree<SavedPane>,
    /// Index of the focused pane among the layout's leaves.
    #[serde(default)]
    pub focused: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    pub name: String,
    pub root: PathBuf,
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    #[serde(default)]
    pub active_tab: usize,
    /// Unix seconds.
    #[serde(default)]
    pub last_active: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedState {
    #[serde(default)]
    pub version: u32,
    /// Index of the workspace the window showed last.
    #[serde(default)]
    pub current: usize,
    #[serde(default)]
    pub workspaces: Vec<SavedWorkspace>,
    /// The files sidebar (⌘B) was showing.
    #[serde(default = "shown")]
    pub sidebar: bool,
    /// Machines dock width in logical pixels (independent of the Files dock).
    #[serde(default = "default_machines_dock_width")]
    pub machines_dock_width: f32,
    /// Files dock width once a user has resized it. When absent, keep the
    /// original local/remote widths until the user actually drags the edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_dock_width: Option<f32>,
    /// Herdr terminal identities intentionally terminated from Machines.
    /// Persisted because terminal.list can retain finished history records.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminated_herdr_sessions: Vec<String>,
}

fn shown() -> bool {
    true
}

fn default_machines_dock_width() -> f32 {
    260.
}

/// A first launch shows the sidebar, as a file from before it was saved does.
impl Default for SavedState {
    fn default() -> Self {
        Self {
            version: 0,
            current: 0,
            workspaces: Vec::new(),
            sidebar: true,
            machines_dock_width: default_machines_dock_width(),
            files_dock_width: None,
            terminated_herdr_sessions: Vec::new(),
        }
    }
}

pub const VERSION: u32 = 2;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
