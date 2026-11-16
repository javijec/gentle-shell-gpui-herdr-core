//! Linux Files bound to one Herdr Pi agent. Remote preview and opt-in
//! revision-checked UTF-8 editing run through the existing injected bridge.
//! This view never uses local Windows filesystem APIs or remote Pi bash.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use crate::features::session::SessionView;
use crate::features::workspaces::SavedRemoteDraft;
use crate::features::git::{RefEntry, RefsSnapshot, WorkTree, StashAction};
use gentle_core::git::refs::{HeadInfo, RefKind, Stash, Submodule, parse_refs, parse_submodules};
use super::refs::{RefsEvent, RefsView, RemoteWorktreeDisplay, RemoteWorktreeAction};
use super::changes::{ChangesEvent, ChangesView};
use super::backend::{
    GitFileAction, GitFileMutation, HerdrGitGuard, WorkspaceBackend,
    WorkspaceEntryAction,
    WorkspaceDirEntry as RemoteEntry, WorkspacePath,
    WorkspaceText as RemotePreview,
};
use gentle_core::git::status::{BranchStatus, StatusEntry as RemoteGitFile};
use super::dock_ui::{file_entry_row, panel_header, panel_shell};
use super::remote_tree::{RemoteTreeRow, visible_rows};
use super::transfers;
use super::tabs::FilePanelTab as RemoteTab;
use super::viewer::MdMode;
use crate::shared::ui;

pub enum RemoteFilesEvent {
    InsertPath(String),
    /// Open the same History pane for the owning remote session.
    OpenHistory,
    /// Open a newly created or existing remote worktree as a Herdr agent.
    OpenWorktreeAgent(String),
    /// Show a Linux file in the same quick-look modal used for native files.
    PeekFile(String),
    /// Open an existing remote Linux file in a normal central editor pane.
    OpenFile(String),
    /// Request a native central diff for a changed Linux file.
    OpenDiff(String),
    /// Copy a regular remote file directly to another selected Herdr agent.
    RelayFile { path: String, target_terminal: String },
    Close,
}

struct RemoteGitBranch {
    name: String,
    current: bool,
    local: bool,
}

struct RemoteGitStash {
    name: String,
    message: String,
    oid: Option<String>,
}

struct RemoteGitSnapshot {
    toplevel: String,
    head_oid: Option<String>,
    current_branch: Option<String>,
    index_token: Option<String>,
    writes_enabled: bool,
    files: Vec<RemoteGitFile>,
    branches: Vec<RemoteGitBranch>,
    tags: Vec<String>,
    stashes: Vec<RemoteGitStash>,
    ref_details: Vec<RefEntry>,
    worktrees: Vec<RemoteWorktreeDisplay>,
    submodules: Vec<Submodule>,
    files_truncated: bool,
    branches_truncated: bool,
    tags_truncated: bool,
    stashes_truncated: bool,
}

/// Parse Linux worktree inventory for display only. Never convert paths
/// to Windows PathBuf, and never offer native filesystem actions for them.
fn parse_remote_worktrees(
    raw: &str, root: &str,
) -> Result<Vec<RemoteWorktreeDisplay>, &'static str> {
    let mut result = Vec::new();
    for block in raw.split("\n\n").filter(|block| !block.trim().is_empty()) {
        let (mut path, mut branch, mut head) = (None, None, None);
        let (mut locked, mut prunable) = (false, false);
        for line in block.lines() {
            if let Some(value) = line.strip_prefix("worktree ") {
                path = Some(value.to_owned());
            } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
                branch = Some(value.to_owned());
            } else if let Some(value) = line.strip_prefix("HEAD ") {
                head = Some(value.to_owned());
            } else if line == "locked" || line.starts_with("locked ") {
                locked = true;
            } else if line == "prunable" || line.starts_with("prunable ") {
                prunable = true;
            }
        }
        let Some(path) = path else { continue; };
        if !path.starts_with('/') || path.contains('\\')
            || path.len() > 2048 || path.chars().any(char::is_control)
        {
            return Err("Invalid remote worktree path");
        }
        if path == root { continue; } // Native view also hides the main worktree.
        result.push(RemoteWorktreeDisplay { path, branch, head, locked, prunable });
        if result.len() > 120 {
            return Err("Too many remote worktrees");
        }
    }
    Ok(result)
}

/// The JSON shape is not trusted just because it came from an RPC bridge.
/// Never turn a returned Linux path into a local Windows file path.
fn parse_git_snapshot(data: &serde_json::Value) -> Result<RemoteGitSnapshot, &'static str> {
    let toplevel = data["toplevel"].as_str()
        .filter(|root| root.starts_with('/') && !root.contains('\\'))
        .ok_or("Invalid remote Git repository path")?
        .to_owned();
    if data["readOnly"].as_bool() != Some(true) {
        return Err("Invalid remote Git snapshot mode");
    }
    let files = data["files"].as_array()
        .ok_or("Missing remote Git status")?
        .iter()
        .take(200)
        .map(|item| {
            let path = item["path"].as_str()
                .filter(|s| !s.is_empty() && s.len() <= 2048
                    && !s.starts_with('/') && !s.contains('\\') && !s.contains(':')
                    && !s.chars().any(char::is_control)
                    && !s.split('/').any(|p| p == "." || p == ".."))
                .ok_or("Invalid Git status file")?;
            let index = item["index"].as_str()
                .filter(|s| s.len() == 1)
                .ok_or("Invalid Git index state")?
                .chars().next().ok_or("Invalid Git index state")?;
            let worktree = item["worktree"].as_str()
                .filter(|s| s.len() == 1)
                .ok_or("Invalid Git worktree state")?
                .chars().next().ok_or("Invalid Git worktree state")?;
            Ok(RemoteGitFile::from_porcelain_v1(path.to_owned(), index, worktree))
        }).collect::<Result<Vec<_>, &'static str>>()?;
    let branches = data["branches"].as_array()
        .ok_or("Missing remote branches")?
        .iter()
        .take(120)
        .map(|item| {
            let name = item["name"].as_str()
                .filter(|s| !s.is_empty() && s.len() <= 512
                    && !s.chars().any(char::is_control))
                .ok_or("Invalid remote branch")?;
            let current = item["current"].as_bool()
                .ok_or("Missing active branch state")?;
            Ok(RemoteGitBranch {
                name: name.to_owned(), current,
                local: item["local"].as_bool().unwrap_or(false),
            })
        }).collect::<Result<Vec<_>, &'static str>>()?;
    // Older running 5D agents omit tags and stashes. Treat omitted fields
    // as unsupported/empty without weakening validation of supplied data.
    let tags = match data.get("tags") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.iter().take(120).map(|item| {
            item["name"].as_str()
                .filter(|s| !s.is_empty() && s.len() <= 512
                    && !s.chars().any(char::is_control))
                .map(str::to_owned)
                .ok_or("Invalid remote Git tag")
        }).collect::<Result<Vec<_>, &'static str>>()?,
        _ => return Err("Invalid remote Git tags"),
    };
    let stashes = match data.get("stashes") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.iter().take(120).map(|item| {
            let name = item["name"].as_str()
                .filter(|s| s.starts_with("stash@{") && s.ends_with('}')
                    && s.len() <= 64 && s[7..s.len()-1].bytes().all(|b| b.is_ascii_digit())
                    && s.len() > 8)
                .ok_or("Invalid remote Git stash name")?;
            let message = item["message"].as_str()
                .filter(|s| s.len() <= 2048 && !s.chars().any(char::is_control))
                .ok_or("Invalid remote Git stash message")?;
            let oid = item["oid"].as_str()
                .filter(|oid| matches!(oid.len(), 40 | 64)
                    && oid.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .map(str::to_owned);
            Ok(RemoteGitStash {
                name: name.to_owned(), message: message.to_owned(), oid,
            })
        }).collect::<Result<Vec<_>, &'static str>>()?,
        _ => return Err("Invalid remote Git stashes"),
    };
    let ref_details = match data.get("refDetails") {
        None => Vec::new(), // Older bridge without extended ref information
        Some(serde_json::Value::String(raw)) if raw.len() <= 1_500_000 => {
            let parsed = parse_refs(raw);
            if parsed.len() > 360 || parsed.iter().any(|entry| {
                entry.name.is_empty() || entry.name.len() > 512
                    || entry.name.chars().any(char::is_control)
            }) {
                return Err("Invalid remote Git reference metadata");
            }
            parsed
        }
        _ => return Err("Invalid remote Git reference metadata"),
    };
    let worktrees = match data.get("worktreeData") {
        None => Vec::new(),
        Some(serde_json::Value::String(raw)) if raw.len() <= 3_000_000 =>
            parse_remote_worktrees(raw, &toplevel)?,
        _ => return Err("Invalid remote worktree inventory"),
    };
    let submodules = match data.get("submoduleData") {
        None => Vec::new(),
        Some(serde_json::Value::String(raw)) if raw.len() <= 500_000 => {
            let parsed = parse_submodules(raw);
            if parsed.len() > 120 || parsed.iter().any(|item| {
                item.path.is_empty() || item.path.len() > 2048
                    || item.path.starts_with('/')
                    || item.path.chars().any(char::is_control)
                    || item.path.split('/').any(|part| part == "." || part == "..")
            }) {
                return Err("Invalid remote submodule inventory");
            }
            parsed
        }
        _ => return Err("Invalid remote submodule inventory"),
    };
    let head_oid = match data.get("headOid") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) if (s.len() == 40 || s.len() == 64)
            && s.bytes().all(|b| b.is_ascii_hexdigit()) => Some(s.to_owned()),
        _ => return Err("Invalid remote Git HEAD"),
    };
    let current_branch = match data.get("currentBranch") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) if !s.is_empty()
            && s.len() <= 512 && !s.chars().any(char::is_control) => Some(s.to_owned()),
        _ => return Err("Invalid remote Git current branch"),
    };
    let index_token = data["indexToken"].as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned);
    let writes_enabled = data["writesEnabled"].as_bool() == Some(true)
        && index_token.is_some()
        && data["indexTruncated"].as_bool() != Some(true);
    Ok(RemoteGitSnapshot {
        toplevel, head_oid, current_branch, index_token, writes_enabled,
        files, branches, tags, stashes, ref_details, worktrees, submodules,
        files_truncated: data["filesTruncated"].as_bool().unwrap_or(false),
        branches_truncated: data["branchesTruncated"].as_bool().unwrap_or(false),
        tags_truncated: data["tagsTruncated"].as_bool().unwrap_or(false),
        stashes_truncated: data["stashesTruncated"].as_bool().unwrap_or(false),
    })
}

/// The shared Changes dialog labels stash messages optional. The remote
/// bridge requires a non-empty `-m` value, so use a deterministic WIP
/// message rather than silently ignoring an empty request.
fn remote_stash_message(raw: &str) -> Option<String> {
    let supplied = raw.trim();
    let message = if supplied.is_empty() {
        "WIP (Gentle Shell)"
    } else {
        supplied
    };
    (message.len() <= 120 && !message.chars().any(char::is_control))
        .then(|| message.to_owned())
}

/// A Git status path is relative to the actual Linux toplevel. Never let
/// a menu event become a Windows path, an absolute path or a traversal.
fn remote_git_file_path(toplevel: &str, relative: &str) -> Option<String> {
    if !toplevel.starts_with('/') || toplevel.contains('\\')
        || toplevel.chars().any(char::is_control)
        || relative.is_empty() || relative.starts_with('/')
        || relative.contains('\\') || relative.contains(':')
        || relative.len() > 2048 || relative.chars().any(char::is_control)
        || relative.split('/').any(|segment| {
            segment.is_empty() || matches!(segment, "." | "..")
        })
    {
        return None;
    }
    Some(format!("{}/{}", toplevel.trim_end_matches('/'), relative))
}

fn shared_remote_worktree(source: &RemoteGitSnapshot) -> WorkTree {
    WorkTree {
        branch: BranchStatus {
            oid: source.head_oid.clone(),
            head: source.current_branch.clone(),
            ..BranchStatus::default()
        },
        entries: source.files.clone(),
    }
}

/// Project a bounded Herdr Git read into the same native RefsSnapshot
/// consumed by RefsView. Linux paths never become native PathBuf objects.
fn shared_remote_refs(source: &RemoteGitSnapshot) -> RefsSnapshot {
    let mut refs = RefsSnapshot {
        head: HeadInfo {
            branch: source.current_branch.clone(),
            sha: source.head_oid.as_ref().map(|oid| oid.chars().take(7).collect()),
            dirty: !source.files.is_empty(),
            ..HeadInfo::default()
        },
        ..RefsSnapshot::default()
    };
    for branch in &source.branches {
        let kind = if branch.local { RefKind::Branch } else { RefKind::Remote };
        let mut entry = source.ref_details.iter()
            .find(|details| details.kind == kind && details.name == branch.name)
            .cloned()
            .unwrap_or_else(|| RefEntry {
                kind, name: branch.name.clone(), current: branch.current,
                upstream: None, ahead: 0, behind: 0, gone: false,
                sha: String::new(), subject: String::new(),
            });
        // The branch list is authoritative about HEAD in this snapshot.
        entry.current = branch.current;
        if branch.local {
            refs.branches.push(entry);
        } else {
            refs.remotes.push(entry);
        }
    }
    refs.tags = source.tags.iter().map(|tag| {
        source.ref_details.iter()
            .find(|entry| entry.kind == RefKind::Tag && entry.name == *tag)
            .cloned()
            .unwrap_or_else(|| RefEntry {
                kind: RefKind::Tag, name: tag.clone(), current: false,
                upstream: None, ahead: 0, behind: 0, gone: false,
                sha: String::new(), subject: String::new(),
            })
    }).collect();
    if let Some(head) = refs.branches.iter().find(|entry| entry.current) {
        refs.head.upstream = head.upstream.clone();
        refs.head.ahead = head.ahead;
        refs.head.behind = head.behind;
    }
    refs.stashes = source.stashes.iter().map(|stash| Stash {
        name: stash.name.clone(), branch: None, message: stash.message.clone(),
    }).collect();
    refs.submodules = source.submodules.clone();
    refs
}

#[cfg(test)]
mod remote_git_snapshot_tests {
    use super::{
        parse_git_snapshot, remote_git_file_path, remote_stash_message,
        shared_remote_refs, shared_remote_worktree,
    };
    use serde_json::json;

    #[test]
    fn empty_stash_message_has_valid_nonempty_default() {
        assert_eq!(remote_stash_message(""), Some("WIP (Gentle Shell)".into()));
        assert_eq!(remote_stash_message("  work in progress  "),
            Some("work in progress".into()));
        assert_eq!(remote_stash_message("bad\nmessage"), None);
        assert_eq!(remote_stash_message(&"a".repeat(121)), None);
    }

    #[test]
    fn git_menu_paths_are_linux_repository_scoped() {
        assert_eq!(
            remote_git_file_path("/srv/workspaces/JaviPi", "src/main.rs"),
            Some("/srv/workspaces/JaviPi/src/main.rs".to_owned()),
        );
        for path in ["", "/etc/passwd", "../outside", "src/../outside",
            "src//main.rs", "src\\main.rs", "C:danger", "a\nb"]
        {
            assert!(remote_git_file_path("/srv/workspaces/JaviPi", path).is_none());
        }
        assert!(remote_git_file_path("C:/repo", "main.rs").is_none());
    }

    #[test]
    fn remote_worktrees_are_display_only_posix_paths() {
        let raw = concat!(
            "worktree /srv/workspaces/JaviPi\nHEAD abc123\nbranch refs/heads/main\n\n",
            "worktree /srv/workspaces/JaviPi.worktrees/test\nHEAD def456\n",
            "branch refs/heads/test\nlocked\n\n",
        );
        let trees = super::parse_remote_worktrees(raw, "/srv/workspaces/JaviPi")
            .expect("valid worktrees");
        assert_eq!(trees.len(), 1);
        assert_eq!(trees[0].branch.as_deref(), Some("test"));
        assert!(trees[0].locked);
        assert_eq!(trees[0].path, "/srv/workspaces/JaviPi.worktrees/test");
        assert!(super::parse_remote_worktrees(
            r"worktree C:\temp\file", "/srv/workspaces/JaviPi",
        ).is_err());
    }

    #[test]
    fn enriches_remote_refs_from_bounded_native_format() {
        let data = json!({
            "toplevel": "/srv/workspaces/JaviPi",
            "readOnly": true,
            "files": [],
            "branches": [{"name":"main","current":true,"local":true}],
            "tags": [{"name":"v1"}],
            "refDetails": concat!(
                "*\0refs/heads/main\0origin/main\0[ahead 2, behind 1]\0abc1234\0feature\n",
                " \0refs/tags/v1\0\0\0def1234\0release\n",
            ),
        });
        let bridge = parse_git_snapshot(&data).expect("valid enriched metadata");
        let refs = shared_remote_refs(&bridge);
        assert_eq!(refs.head.upstream.as_deref(), Some("origin/main"));
        assert_eq!((refs.head.ahead, refs.head.behind), (2, 1));
        assert_eq!(refs.tags[0].subject, "release");
    }

    #[test]
    fn typed_herdr_snapshots_feed_the_same_native_refs_and_changes_views() {
        let data = json!({
            "toplevel": "/srv/workspaces/JaviPi",
            "readOnly": true,
            "writesEnabled": true,
            "headOid": "a".repeat(40),
            "currentBranch": "main",
            "indexToken": "b".repeat(64),
            "files": [
                {"path": "src/main.rs", "index": "M", "worktree": "M"},
                {"path": "new.txt", "index": "?", "worktree": "?"}
            ],
            "branches": [
                {"name": "main", "current": true, "local": true},
                {"name": "dev", "current": false, "local": true},
                {"name": "origin/main", "current": false, "local": false}
            ],
            "tags": [{"name": "v1.0"}],
            "stashes": [{"name": "stash@{0}", "message": "On main: save"}]
        });
        let bridge = parse_git_snapshot(&data).expect("valid agent snapshot");
        let changes = shared_remote_worktree(&bridge);
        let refs = shared_remote_refs(&bridge);
        assert_eq!(changes.entries.len(), 2);
        assert_eq!(changes.branch.head.as_deref(), Some("main"));
        assert_eq!((changes.entries[0].staged(), changes.entries[0].unstaged()),
            (Some('M'), Some('M')));
        assert_eq!(refs.branches.len(), 2);
        assert_eq!(refs.remotes.len(), 1);
        assert_eq!(refs.tags.len(), 1);
        assert_eq!(refs.stashes.len(), 1);
        assert_eq!(refs.head.branch.as_deref(), Some("main"));
        assert_eq!(refs.remotes[0].name, "origin/main");
        assert_eq!(refs.stashes[0].name, "stash@{0}");
    }

    #[test]
    fn tags_and_stashes_accept_new_snapshots_and_validate_records() {
        let mut value = json!({
            "toplevel": "/srv/workspaces/JaviPi",
            "readOnly": true,
            "files": [],
            "branches": [],
            "tags": [{"name":"v1.0"}],
            "stashes": [{"name":"stash@{0}", "message":"On main: my edit"}],
            "tagsTruncated": true,
            "stashesTruncated": false,
        });
        let parsed = parse_git_snapshot(&value).expect("valid refs");
        assert_eq!(parsed.tags, vec!["v1.0"]);
        assert_eq!(parsed.stashes[0].name, "stash@{0}");
        assert!(parsed.tags_truncated);
        assert!(!parsed.stashes_truncated);
        value["stashes"][0]["name"] = json!("stash@{nope}");
        assert!(parse_git_snapshot(&value).is_err());
        value["stashes"] = json!([]);
        value["tags"] = json!({"malformed":true});
        assert!(parse_git_snapshot(&value).is_err());
    }

    #[test]
    fn supports_guarded_5d_snapshots_and_old_read_only_bridge() {
        let mut value = json!({
            "toplevel": "/srv/workspaces/JaviPi",
            "readOnly": true,
            "writesEnabled": true,
            "headOid": "a".repeat(40),
            "currentBranch": "main",
            "indexToken": "b".repeat(64),
            "indexTruncated": false,
            "files": [{ "path": "tracked.txt", "index": "M", "worktree": " " }],
            "branches": [{ "name": "main", "current": true, "local": true }],
        });
        let result = parse_git_snapshot(&value).expect("valid 5D snapshot");
        assert!(result.writes_enabled);
        assert_eq!(result.index_token.as_deref().map(str::len), Some(64));
        assert!(result.branches[0].local);
        value.as_object_mut().expect("object").remove("indexToken");
        assert!(!parse_git_snapshot(&value).expect("missing token").writes_enabled);
        value["readOnly"] = json!(false);
        assert!(parse_git_snapshot(&value).is_err());
    }

    #[test]
    fn rejects_paths_that_escape_linux_repository() {
        let value = json!({
            "toplevel": "/srv/workspaces/JaviPi",
            "readOnly": true,
            "files": [{ "path": "../outside", "index": "M", "worktree": " " }],
            "branches": [],
        });
        assert!(parse_git_snapshot(&value).is_err());
    }
}

const MAX_EDIT_BYTES: usize = 2 * 1024 * 1024;
const CONTEXT: &str = "GentleRemoteFileView";
const TREE_CONTEXT: &str = "GentleRemoteFilesTree";
const SAVE_HINT: &str = if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" };

actions!(herdr_file_view, [
    Save, TreeUp, TreeDown, TreeLeft, TreeRight, TreeOpen,
    TreeRename, TreeCopy, TreeCut, TreePaste,
]);

pub struct RemoteFilesView {
    session: Entity<SessionView>,
    machine: String,
    root: String,
    current: String,
    /// A central file pane has a fixed remote file path; None is the
    /// right-hand directory browser. Never convert this into Windows PathBuf.
    pane_path: Option<String>,
    /// The right dock uses Files/Changes/Branches, never the local Git repo.
    tab: RemoteTab,
    git: Option<RemoteGitSnapshot>,
    git_loading: bool,
    git_error: Option<String>,
    git_generation: u64,
    git_busy: bool,
    git_feedback: Option<String>,
    git_branch_name: Entity<InputState>,
    refs: Entity<RefsView>,
    _refs_events: Subscription,
    changes: Entity<ChangesView>,
    _changes_events: Subscription,
    git_confirm_create: Option<String>,
    git_confirm_switch: Option<String>,
    git_confirm_stash: Option<(String, StashAction)>,
    git_confirm_network: Option<String>,
    git_confirm_worktree: Option<(RemoteWorktreeAction, String)>,
    _git_events: Vec<Subscription>,
    focus: FocusHandle,
    tree_children: HashMap<String, Vec<RemoteEntry>>,
    tree_rows: Rc<Vec<RemoteTreeRow>>,
    tree_scroll: UniformListScrollHandle,
    tree_expanded: HashSet<String>,
    tree_loading: HashSet<String>,
    show_ignored: bool,
    tree_selected_directory: Option<String>,
    tree_selected_entry: Option<(String, bool)>,
    file_clipboard: Option<(String, bool)>,
    entry_name: Entity<InputState>,
    entry_busy: bool,
    transfer_busy: bool,
    transfer_feedback: Option<String>,
    /// Human-readable destination and exact Herdr terminal address.
    transfer_targets: Vec<(String, String)>,
    trash_confirm: Option<String>,
    last_trash: Option<(String, String)>,
    create_name: Entity<InputState>,
    creating: bool,
    preview: Option<RemotePreview>,
    editor: Option<Entity<EditorState>>,
    /// A persisted unsaved buffer is applied only to its original path and
    /// always retains the original server revision for conflict detection.
    pending_draft: Option<SavedRemoteDraft>,
    restored_revision: Option<String>,
    /// Markdown uses the same Edit / Split / Preview mode as FileView.
    markdown: Option<(MdMode, Entity<TextViewState>)>,
    _markdown_debounce: Task<()>,
    _editor_events: Option<Subscription>,
    saving: bool,
    loading: bool,
    error: Option<String>,
    truncated: bool,
    generation: u64,
    _wait_for_agent: Task<()>,
}

impl EventEmitter<RemoteFilesEvent> for RemoteFilesView {}

#[cfg(test)]
fn remote_child(directory: &str, name: &str) -> String {
    if directory == "/" {
        format!("/{name}")
    } else {
        format!("{directory}/{name}")
    }
}

fn remote_parent(root: &str, directory: &str) -> Option<String> {
    if directory == root {
        return None;
    }
    let (parent, _) = directory.rsplit_once('/')?;
    Some(if parent.is_empty() { "/".into() } else { parent.into() })
        .filter(|candidate: &String| {
            candidate.as_str() == root || (root == "/"
                && candidate.starts_with('/'))
                || candidate.starts_with(&(root.to_owned() + "/"))
        })
}

/// Retry transport startup failures, not filesystem permission/path errors.
fn retryable_remote_loading_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("waiting for this remote agent to connect")
        || error.contains("rpc bridge is not connected")
        || error.contains("rpc bridge exited before replying")
        || error.contains("remote agent is disconnected")
        || error.contains("connection interrupted")
        || error.contains("connection reset")
}

fn friendly_error(error: &str) -> String {
    let low = error.to_lowercase();
    if low.contains("unknown command") || low.contains("unsupported command")
        || low.contains("unrecognized command") || low.contains("unknown type")
        || low.contains("unknown rpc") || low.contains("not implemented")
    {
        "This running agent has an older Herdr bridge. Open a new remote agent to enable the requested file operation; the existing agent is preserved.".into()
    } else {
        error.into()
    }
}

impl RemoteFilesView {
    /// Use the same save shortcut as local FileView, scoped to the
    /// remote editor's focused UI. The action always saves through Herdr.
    pub fn bind_keys(cx: &mut App) {
        let key = if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" };
        let copy = if cfg!(target_os = "macos") { "cmd-c" } else { "ctrl-c" };
        let cut = if cfg!(target_os = "macos") { "cmd-x" } else { "ctrl-x" };
        let paste = if cfg!(target_os = "macos") { "cmd-v" } else { "ctrl-v" };
        cx.bind_keys([
            KeyBinding::new(key, Save, Some(CONTEXT)),
            KeyBinding::new("up", TreeUp, Some(TREE_CONTEXT)),
            KeyBinding::new("down", TreeDown, Some(TREE_CONTEXT)),
            KeyBinding::new("left", TreeLeft, Some(TREE_CONTEXT)),
            KeyBinding::new("right", TreeRight, Some(TREE_CONTEXT)),
            KeyBinding::new("enter", TreeOpen, Some(TREE_CONTEXT)),
            KeyBinding::new("f2", TreeRename, Some(TREE_CONTEXT)),
            KeyBinding::new(copy, TreeCopy, Some(TREE_CONTEXT)),
            KeyBinding::new(cut, TreeCut, Some(TREE_CONTEXT)),
            KeyBinding::new(paste, TreePaste, Some(TREE_CONTEXT)),
        ]);
    }

    fn save_shortcut(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save_edit(cx);
    }

    fn select_tree_row(
        &mut self, next: bool, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.pane_path.is_some() || self.tree_rows.is_empty() { return; }
        let current = self.tree_selected_entry.as_ref().and_then(|(path, _)| {
            self.tree_rows.iter().position(|node| &node.path == path)
        });
        let index = match (current, next) {
            (None, true) => 0,
            (None, false) => self.tree_rows.len() - 1,
            (Some(i), true) => (i + 1).min(self.tree_rows.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        let node = &self.tree_rows[index];
        let path = node.path.clone();
        let is_directory = node.entry.kind == "directory";
        let name = node.entry.name.clone();
        self.tree_selected_entry = Some((path.clone(), is_directory));
        self.tree_selected_directory = if is_directory {
            Some(path)
        } else {
            remote_parent(&self.root, &path)
        };
        self.entry_name.update(cx, |field, cx| {
            field.set_value(name, window, cx);
        });
        self.tree_scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn tree_up(&mut self, _: &TreeUp, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tree_row(false, window, cx);
    }
    fn tree_down(&mut self, _: &TreeDown, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tree_row(true, window, cx);
    }
    fn tree_open(&mut self, _: &TreeOpen, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((path, directory)) = self.tree_selected_entry.clone() {
            if directory {
                self.toggle_remote_directory(path, cx);
            } else {
                cx.emit(RemoteFilesEvent::OpenFile(path));
            }
        }
    }
    fn tree_left(&mut self, _: &TreeLeft, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, directory)) = self.tree_selected_entry.clone() else { return; };
        if directory && self.tree_expanded.contains(&path) {
            self.toggle_remote_directory(path, cx);
        } else if let Some(parent) = remote_parent(&self.root, &path)
            && let Some(i) = self.tree_rows.iter().position(|node| node.path == parent)
        {
            let node = &self.tree_rows[i];
            let name = node.entry.name.clone();
            self.tree_selected_entry = Some((parent.clone(), true));
            self.tree_selected_directory = Some(parent);
            self.tree_scroll.scroll_to_item(i, ScrollStrategy::Nearest);
            self.entry_name.update(cx, |field, cx| {
                field.set_value(name, window, cx);
            });
            cx.notify();
        }
    }
    fn tree_right(&mut self, _: &TreeRight, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((path, true)) = self.tree_selected_entry.clone()
            && !self.tree_expanded.contains(&path)
        {
            self.toggle_remote_directory(path, cx);
        }
    }
    fn tree_rename(&mut self, _: &TreeRename, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree_selected_entry.is_some() {
            self.entry_name.update(cx, |field, cx| field.focus(window, cx));
        }
    }
    fn tree_copy(&mut self, _: &TreeCopy, _: &mut Window, cx: &mut Context<Self>) {
        self.clipboard_remote_entry(false, cx);
    }
    fn tree_cut(&mut self, _: &TreeCut, _: &mut Window, cx: &mut Context<Self>) {
        self.clipboard_remote_entry(true, cx);
    }
    fn tree_paste(&mut self, _: &TreePaste, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_remote_entry(cx);
    }

    pub fn new(
        session: Entity<SessionView>,
        machine: String,
        root: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let git_branch_name = cx.new(|cx| {
            InputState::new(window, cx).placeholder("New local branch name")
        });
        let create_name = cx.new(|cx| {
            InputState::new(window, cx).placeholder("New file or folder name")
        });
        let entry_name = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Rename selected entry")
        });
        let refs = cx.new(|cx| RefsView::new_remote(window, cx));
        let refs_events = cx.subscribe(&refs, |this, _, event: &RefsEvent, cx| {
            match event {
                RefsEvent::RemoteOpenHistory =>
                    cx.emit(RemoteFilesEvent::OpenHistory),
                RefsEvent::RemoteOpenWorktree(path) =>
                    cx.emit(RemoteFilesEvent::OpenWorktreeAgent(path.clone())),
                RefsEvent::SwitchRemoteBranch(branch) =>
                    this.git_switch_branch(branch.clone(), cx),
                RefsEvent::RemoteStash { name, action } =>
                    this.git_stash_action(name.clone(), *action, cx),
                RefsEvent::RemoteWorktree { action, branch, path } =>
                    this.git_worktree_action(*action, branch.clone(), path.clone(), cx),
                _ => {}
            }
        });
        let changes = cx.new(|cx| ChangesView::new_remote(window, cx));
        let changes_events = cx.subscribe(&changes, |this, _, event: &ChangesEvent, cx| {
            match event {
                ChangesEvent::RemoteStage { path, index, worktree, stage } => {
                    this.git_file_action(
                        if *stage { GitFileAction::Stage } else { GitFileAction::Unstage },
                        path.clone(), *index, *worktree, cx,
                    );
                }
                ChangesEvent::RemoteOpenDiff(path) => {
                    if let Some(absolute) = this.git.as_ref().and_then(|snapshot| {
                        remote_git_file_path(&snapshot.toplevel, path)
                    }) {
                        cx.emit(RemoteFilesEvent::OpenDiff(absolute));
                    }
                }
                ChangesEvent::RemoteOpenFile(path) => {
                    if let Some(absolute) = this.git.as_ref().and_then(|snapshot| {
                        remote_git_file_path(&snapshot.toplevel, path)
                    }) {
                        cx.emit(RemoteFilesEvent::OpenFile(absolute));
                    }
                }
                ChangesEvent::RemoteCommit { summary, description, amend, push } => {
                    this.git_commit_message(
                        summary.clone(), description.clone(), *amend, *push, cx,
                    );
                }
                ChangesEvent::RemoteGenerateMessage => {
                    this.git_generate_message(cx);
                }
                ChangesEvent::RemoteLastCommitMessage => {
                    this.git_load_last_commit_message(cx);
                }
                ChangesEvent::RemoteBulkStage { stage } => {
                    this.git_bulk_stage(*stage, cx);
                }
                ChangesEvent::RemoteStashPush(message) => {
                    this.git_stash_push(message.clone(), cx);
                }
                ChangesEvent::RemoteDiscard { path, index, worktree, staged } => {
                    this.git_discard(path.clone(), *index, *worktree, *staged, cx);
                }
                _ => {}
            }
        });
        let git_events = vec![
            cx.subscribe(&git_branch_name, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event { cx.notify(); }
            }),
            cx.subscribe(&create_name, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event { cx.notify(); }
            }),
            cx.subscribe(&entry_name, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event { cx.notify(); }
            }),
        ];
        let mut view = Self {
            session,
            machine,
            current: root.clone(),
            root,
            pane_path: None,
            tab: RemoteTab::Files,
            git: None,
            git_loading: false,
            git_error: None,
            git_generation: 0,
            git_busy: false,
            git_feedback: None,
            git_branch_name,
            refs,
            _refs_events: refs_events,
            changes,
            _changes_events: changes_events,
            git_confirm_create: None,
            git_confirm_switch: None,
            git_confirm_stash: None,
            git_confirm_network: None,
            git_confirm_worktree: None,
            _git_events: git_events,
            focus: cx.focus_handle(),
            tree_children: HashMap::new(),
            tree_rows: Rc::new(Vec::new()),
            tree_scroll: UniformListScrollHandle::new(),
            tree_expanded: HashSet::new(),
            tree_loading: HashSet::new(),
            show_ignored: false,
            tree_selected_directory: None,
            tree_selected_entry: None,
            file_clipboard: None,
            entry_name,
            entry_busy: false,
            transfer_busy: false,
            transfer_feedback: None,
            transfer_targets: Vec::new(),
            trash_confirm: None,
            last_trash: None,
            create_name,
            creating: false,
            preview: None,
            editor: None,
            pending_draft: None,
            restored_revision: None,
            markdown: None,
            _markdown_debounce: Task::ready(()),
            _editor_events: None,
            saving: false,
            loading: false,
            error: None,
            truncated: false,
            generation: 0,
            _wait_for_agent: Task::ready(()),
        };
        if super::backend::remote_fs_trace_enabled() {
            eprintln!(
                "[herdr-fs-trace] UI view created machine={} root={} rpc_ready={}",
                view.machine,
                view.root,
                view.session.read(cx).remote_fs_process().is_some(),
            );
        }
        if view.root.is_empty() {
            view.error = Some(
                "This restored agent has no known remote working directory yet. Refresh after Herdr discovers its terminal, or open a new agent."
                    .into(),
            );
            return view;
        }
        view.load_directory(view.root.clone(), cx);
        // A newly launched Pi can expose a process before the bridge is
        // ready. Retry only transient transport errors, including failures
        // that arrive asynchronously after the initial directory request.
        view._wait_for_agent = cx.spawn_in(window, async move |this, cx| {
            for _ in 0..45 {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let finished = this.update_in(cx, |this, window, cx| {
                    if this.pane_path.is_some() {
                        if this.preview.is_some() {
                            return true;
                        }
                    } else if this.tree_children.contains_key(&this.current) {
                        return true;
                    }
                    if this.loading {
                        return false;
                    }
                    if !this.error.as_deref().is_some_and(retryable_remote_loading_error) {
                        return true;
                    }
                    if this.session.read(cx).remote_fs_process().is_none() {
                        return false;
                    }
                    if let Some(path) = this.pane_path.clone() {
                        this.open(path, false, window, cx);
                    } else {
                        this.load_directory(this.current.clone(), cx);
                    }
                    false
                });
                if !matches!(finished, Ok(false)) {
                    break;
                }
            }
        });
        view
    }

    pub fn matches(&self, session: &Entity<SessionView>, root: &str) -> bool {
        &self.session == session && self.root == root && self.pane_path.is_none()
    }

    /// Build a central source-file pane with the same editor control as the
    /// Windows FileView. It still speaks to this one agent via Pi RPC.
    pub fn new_pane(
        session: Entity<SessionView>,
        machine: String,
        root: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(session, machine, root, window, cx);
        view.pane_path = Some(path.clone());
        if view.session.read(cx).remote_fs_process().is_some() {
            view.open(path, false, window, cx);
        }
        view
    }

    pub fn remote_path(&self) -> Option<&str> {
        self.pane_path.as_deref()
    }

    pub fn draft_snapshot(&self, cx: &App) -> Option<SavedRemoteDraft> {
        let path = self.pane_path.as_deref()?;
        if let Some(saved) = self.pending_draft.as_ref() {
            return (saved.path == path).then(|| saved.clone());
        }
        let (Some(preview), Some(editor)) = (&self.preview, &self.editor) else {
            return None;
        };
        let content = editor.read(cx).text().to_string();
        let revision = self.restored_revision.as_deref()
            .or(preview.revision.as_deref())?;
        if content == preview.content && self.restored_revision.is_none() {
            return None;
        }
        if content.len() > MAX_EDIT_BYTES || content.contains('\0') ||
            revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        Some(SavedRemoteDraft {
            path: path.to_owned(), content, revision: revision.to_owned(),
        })
    }

    /// Called only for the original recovered remote pane; the draft stays
    /// local until the owning Herdr process has returned the real file.
    pub fn restore_draft(&mut self, draft: SavedRemoteDraft, cx: &mut Context<Self>) {
        if self.pane_path.as_deref() != Some(draft.path.as_str())
            || draft.content.len() > MAX_EDIT_BYTES
            || draft.content.contains('\0')
            || draft.revision.len() != 64
            || !draft.revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            self.error = Some("Skipped an invalid recovered remote draft.".into());
            cx.notify();
            return;
        }
        self.pending_draft = Some(draft);
        cx.notify();
    }

    fn apply_pending_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.pending_draft.take() else { return };
        let (Some(preview), Some(editor)) = (&self.preview, &self.editor) else {
            self.pending_draft = Some(draft);
            return;
        };
        if preview.path != draft.path {
            self.pending_draft = Some(draft);
            return;
        }
        let changed_while_closed =
            preview.revision.as_deref() != Some(draft.revision.as_str());
        self.restored_revision = Some(draft.revision);
        editor.update(cx, |editor, cx| {
            editor.set_value(draft.content, window, cx);
        });
        if changed_while_closed {
            self.error = Some(
                "Remote file changed while GPUI was closed. Your local draft is restored,                  but saving requires its original revision. Copy or reconcile your edits first."
                    .into(),
            );
        }
    }

    pub fn file_title(&self, cx: &App) -> SharedString {
        let Some(path) = self.pane_path.as_ref() else {
            return "Remote Files".into();
        };
        let filename = path.rsplit('/').next().unwrap_or(path);
        let dirty = if self.has_unsaved(cx) { " •" } else { "" };
        format!("{filename}{dirty} · {}", self.machine).into()
    }

    pub fn has_unsaved(&self, cx: &App) -> bool {
        self.pending_draft.is_some() || self.saving
            || self.editor.as_ref().zip(self.preview.as_ref()).is_some_and(
            |(editor, preview)| *editor.read(cx).text() != preview.content
        )
    }

    pub fn session(&self) -> &Entity<SessionView> {
        &self.session
    }

    /// Preserve the editor when the user switches machines or agents.
    /// A draft is private to this view and can only save through its own
    /// original SessionView. Unchanged editor mode is retained as well.
    pub fn has_draft(&self) -> bool {
        self.editor.is_some() || self.saving || self.transfer_busy
            || self.pending_draft.is_some()
    }

    pub fn set_transfer_targets(
        &mut self, targets: Vec<(String, String)>, cx: &mut Context<Self>,
    ) {
        if self.transfer_targets != targets {
            self.transfer_targets = targets;
            cx.notify();
        }
    }

    /// Navigation must not silently discard a typed edit.
    fn can_leave_preview(&mut self, cx: &mut Context<Self>) -> bool {
        if self.saving {
            self.error = Some("Save is in progress. Keep this file open.".into());
            cx.notify();
            return false;
        }
        if self.editor.as_ref().zip(self.preview.as_ref())
            .is_some_and(|(editor, preview)| *editor.read(cx).text() != preview.content)
        {
            self.error = Some("Unsaved remote edits. Save or Cancel before navigating.".into());
            cx.notify();
            return false;
        }
        true
    }

    fn load_directory(&mut self, directory: String, cx: &mut Context<Self>) {
        if !self.can_leave_preview(cx) { return; }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let trace = super::backend::remote_fs_trace_enabled();
        let started = std::time::Instant::now();
        if trace {
            eprintln!(
                "[herdr-fs-trace] UI load start machine={} path={} generation={generation}",
                self.machine, directory,
            );
        }
        self.current = directory.clone();
        self.preview = None;
        self.editor = None;
        self.markdown = None;
        self._editor_events = None;
        self.tree_children.clear();
        self.tree_expanded.clear();
        self.tree_loading.clear();
        self.tree_selected_directory = None;
        self.tree_selected_entry = None;
        self.trash_confirm = None;
        self.last_trash = None;
        self.rebuild_remote_rows();
        self.error = None;
        self.truncated = false;
        self.loading = true;
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.loading = false;
            self.error = Some("Waiting for this remote agent to connect…".into());
            if trace {
                eprintln!(
                    "[herdr-fs-trace] UI load waiting path={} rpc_ready=false",
                    self.current,
                );
            }
            cx.notify();
            return;
        };
        let response = WorkspaceBackend::herdr(self.root.clone(), process)
            .list(WorkspacePath::Herdr(directory), self.show_ignored);
        cx.spawn(async move |this, cx| {
            let response = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match response {
                    Ok(listing) => {
                        this.tree_children.insert(this.current.clone(), listing.entries);
                        this.rebuild_remote_rows();
                        this.truncated = listing.truncated;
                    }
                    Err(error) => this.error = Some(friendly_error(&error.to_string())),
                }
                if trace {
                    eprintln!(
                        "[herdr-fs-trace] UI load finished path={} generation={generation} elapsed_ms={} rows={} error={}",
                        this.current,
                        started.elapsed().as_millis(),
                        this.tree_rows.len(),
                        this.error.as_deref().unwrap_or("none"),
                    );
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn rebuild_remote_rows(&mut self) {
        self.tree_rows = Rc::new(visible_rows(
            &self.current, &self.tree_children, &self.tree_expanded, &self.tree_loading,
        ));
    }

    fn toggle_ignored(&mut self, cx: &mut Context<Self>) {
        if !self.can_leave_preview(cx) {
            return;
        }
        self.show_ignored = !self.show_ignored;
        self.load_directory(self.current.clone(), cx);
    }

    fn refresh_tree_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        self.tree_children.remove(&folder);
        self.tree_expanded.remove(&folder);
        self.toggle_remote_directory(folder, cx);
    }

    fn clipboard_remote_entry(&mut self, cut: bool, cx: &mut Context<Self>) {
        if let Some((path, _directory)) = &self.tree_selected_entry {
            self.file_clipboard = Some((path.clone(), cut));
            self.error = None;
            cx.notify();
        }
    }

    fn trash_remote_entry(&mut self, cx: &mut Context<Self>) {
        if self.entry_busy || self.creating || self.root == "/" || self.pane_path.is_some() {
            return;
        }
        let Some((selected, _)) = self.tree_selected_entry.clone() else { return; };
        if self.trash_confirm.as_deref() != Some(selected.as_str()) {
            self.trash_confirm = Some(selected);
            cx.notify();
            return;
        }
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.error = Some("Remote agent is disconnected".into());
            cx.notify();
            return;
        };
        self.trash_confirm = None;
        self.entry_busy = true;
        let parent = remote_parent(&self.root, &selected)
            .unwrap_or_else(|| self.root.clone());
        let generation = self.generation;
        let operation = WorkspaceBackend::herdr(self.root.clone(), process)
            .trash(WorkspacePath::Herdr(selected.clone()));
        cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                this.entry_busy = false;
                match result {
                    Ok(receipt) => {
                        if !matches!(&receipt.path, WorkspacePath::Herdr(path)
                            if path == &selected)
                        {
                            this.error = Some("Unexpected remote Trash result".into());
                            cx.notify();
                            return;
                        }
                        if let Some(token) = receipt.token {
                            this.last_trash = Some((token, selected.clone()));
                        }
                        this.tree_selected_entry = None;
                        this.file_clipboard = None;
                        this.tree_children.remove(&selected);
                        this.tree_expanded.remove(&selected);
                        this.refresh_tree_folder(parent.clone(), cx);
                    }
                    Err(error) => this.error = Some(friendly_error(&error)),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn restore_remote_entry(&mut self, cx: &mut Context<Self>) {
        if self.entry_busy || self.creating || self.root == "/" {
            return;
        }
        let Some((token, original)) = self.last_trash.clone() else { return; };
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.error = Some("Remote agent is disconnected".into());
            cx.notify();
            return;
        };
        self.entry_busy = true;
        let generation = self.generation;
        let parent = remote_parent(&self.root, &original)
            .unwrap_or_else(|| self.root.clone());
        let operation = WorkspaceBackend::herdr(self.root.clone(), process)
            .restore(token.clone());
        cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                this.entry_busy = false;
                match result {
                    Ok(_) => {
                        this.last_trash = None;
                        this.refresh_tree_folder(parent.clone(), cx);
                    }
                    Err(error) => this.error = Some(friendly_error(&error)),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn rename_remote_entry(&mut self, cx: &mut Context<Self>) {
        let new_name = self.entry_name.read(cx).value().trim().to_owned();
        if !new_name.is_empty() {
            self.run_remote_entry(WorkspaceEntryAction::Rename(new_name), cx);
        }
    }

    fn paste_remote_entry(&mut self, cx: &mut Context<Self>) {
        let Some((source, cut)) = self.file_clipboard.clone() else { return; };
        let directory = self.tree_selected_directory.clone()
            .unwrap_or_else(|| self.root.clone());
        self.run_remote_entry_from(
            source,
            if cut {
                WorkspaceEntryAction::Move(WorkspacePath::Herdr(directory))
            } else {
                WorkspaceEntryAction::CopyTo(WorkspacePath::Herdr(directory))
            },
            cx,
        );
    }

    fn run_remote_entry(
        &mut self, action: WorkspaceEntryAction, cx: &mut Context<Self>,
    ) {
        let Some((source, _)) = &self.tree_selected_entry else { return; };
        self.run_remote_entry_from(source.clone(), action, cx);
    }

    fn run_remote_entry_from(
        &mut self, source: String, action: WorkspaceEntryAction,
        cx: &mut Context<Self>,
    ) {
        if self.entry_busy || self.creating || self.root == "/" || self.pane_path.is_some() {
            return;
        }
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.error = Some("Remote agent is disconnected".into());
            cx.notify();
            return;
        };
        let source_parent = remote_parent(&self.root, &source)
            .unwrap_or_else(|| self.root.clone());
        let dest_parent = match &action {
            WorkspaceEntryAction::Move(WorkspacePath::Herdr(dir))
                | WorkspaceEntryAction::CopyTo(WorkspacePath::Herdr(dir)) =>
                dir.clone(),
            _ => source_parent.clone(),
        };
        let should_clear_clipboard = matches!(
            &action, WorkspaceEntryAction::Rename(_) | WorkspaceEntryAction::Move(_)
        );
        let operation = WorkspaceBackend::herdr(self.root.clone(), process)
            .entry(WorkspacePath::Herdr(source.clone()), action);
        self.entry_busy = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                this.entry_busy = false;
                match result {
                    Ok(_) => {
                        let stale_prefix = format!("{source}/");
                        this.tree_children.retain(|path, _| {
                            path != &source && !path.starts_with(&stale_prefix)
                        });
                        this.tree_expanded.retain(|path| {
                            path != &source && !path.starts_with(&stale_prefix)
                        });
                        if this.tree_selected_entry.as_ref()
                            .is_some_and(|(path, _)| path == &source)
                        {
                            this.tree_selected_entry = None;
                        }
                        if this.tree_selected_directory.as_deref() == Some(source.as_str()) {
                            this.tree_selected_directory = None;
                        }
                        if should_clear_clipboard {
                            this.file_clipboard = None;
                        }
                        this.refresh_tree_folder(source_parent.clone(), cx);
                        if dest_parent != source_parent {
                            this.refresh_tree_folder(dest_parent.clone(), cx);
                        }
                        this.rebuild_remote_rows();
                    }
                    Err(error) => this.error = Some(friendly_error(&error)),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Upload a user-selected Windows/macOS/Linux regular file to the
    /// selected agent's directory. No OS path is ever sent to the remote Pi.
    fn upload_local_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.transfer_busy || self.root == "/" { return; }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true, directories: false, multiple: false,
            prompt: Some("Upload to remote project".into()),
        });
        let root = self.root.clone();
        let parent = self.tree_selected_directory.clone()
            .unwrap_or_else(|| root.clone());
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = picker.await else { return };
            let Some(local) = paths.pop() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.transfer_busy { return; }
                let Some(process) = this.session.read(cx).remote_fs_process() else {
                    this.error = Some("Remote agent is disconnected; upload cancelled.".into());
                    cx.notify();
                    return;
                };
                this.transfer_busy = true;
                this.transfer_feedback = None;
                this.error = None;
                let task = cx.background_executor().spawn(
                    transfers::upload(root, process, parent.clone(), local),
                );
                cx.spawn_in(window, async move |this, cx| {
                    let result = task.await;
                    let _ = this.update_in(cx, |this, _, cx| {
                        this.transfer_busy = false;
                        match result {
                            Ok(path) => {
                                this.transfer_feedback = Some(format!(
                                    "Uploaded to remote agent: {path}"
                                ));
                                this.refresh_tree_folder(parent, cx);
                            }
                            Err(error) => this.error = Some(error),
                        }
                        cx.notify();
                    });
                }).detach();
                cx.notify();
            });
        }).detach();
    }

    /// Choose a local directory explicitly. The downloader verifies every
    /// block and uses create_new: it never overwrites local work.
    fn download_remote_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.transfer_busy { return; }
        let path = self.pane_path.clone().or_else(|| {
            self.tree_selected_entry.as_ref()
                .and_then(|(path, is_directory)| (!*is_directory).then(|| path.clone()))
        });
        let Some(path) = path else {
            self.error = Some("Select a regular remote file to download.".into());
            cx.notify();
            return;
        };
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false, directories: true, multiple: false,
            prompt: Some("Download remote file to local directory".into()),
        });
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut dirs))) = picker.await else { return };
            let Some(destination) = dirs.pop() else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.transfer_busy { return; }
                let Some(process) = this.session.read(cx).remote_fs_process() else {
                    this.error = Some("Remote agent is disconnected; download cancelled.".into());
                    cx.notify();
                    return;
                };
                this.transfer_busy = true;
                this.transfer_feedback = None;
                this.error = None;
                let task = cx.background_executor().spawn(
                    transfers::download(root, process, path, destination),
                );
                cx.spawn_in(window, async move |this, cx| {
                    let result = task.await;
                    let _ = this.update_in(cx, |this, _, cx| {
                        this.transfer_busy = false;
                        match result {
                            Ok(path) => this.transfer_feedback = Some(format!(
                                "Downloaded to {}", path.display()
                            )),
                            Err(error) => this.error = Some(error),
                        }
                        cx.notify();
                    });
                }).detach();
                cx.notify();
            });
        }).detach();
    }

    fn relay_selected_file(&mut self, target_terminal: String, cx: &mut Context<Self>) {
        if self.transfer_busy { return; }
        let Some((path, false)) = self.tree_selected_entry.clone() else { return };
        if !self.transfer_targets.iter().any(|(_, terminal)| terminal == &target_terminal) {
            self.error = Some("Target agent is no longer available.".into());
            cx.notify();
            return;
        }
        cx.emit(RemoteFilesEvent::RelayFile { path, target_terminal });
    }

    fn create_remote_entry(
        &mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.creating || self.entry_busy || self.root == "/" || self.pane_path.is_some() {
            return;
        }
        let name = self.create_name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return;
        }
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.error = Some("Remote agent is not connected".into());
            cx.notify();
            return;
        };
        let parent = self.tree_selected_directory.clone()
            .unwrap_or_else(|| self.root.clone());
        let operation = WorkspaceBackend::herdr(self.root.clone(), process)
            .create(WorkspacePath::Herdr(parent.clone()), name, directory);
        self.creating = true;
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.creating = false;
                match result {
                    Ok(_) => {
                        this.create_name.update(cx, |field, cx| {
                            field.set_value(String::new(), window, cx)
                        });
                        // Preserve all expanded folders; only re-query the
                        // parent that received this file or directory.
                        this.refresh_tree_folder(parent.clone(), cx);
                    }
                    Err(error) => this.error = Some(friendly_error(&error)),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Expand a directory without replacing the selected project root or
    /// closing a central editor. Children are loaded only when requested.
    fn toggle_remote_directory(&mut self, path: String, cx: &mut Context<Self>) {
        if self.tree_expanded.remove(&path) {
            self.rebuild_remote_rows();
            cx.notify();
            return;
        }
        self.tree_expanded.insert(path.clone());
        if self.tree_children.contains_key(&path) || self.tree_loading.contains(&path) {
            self.rebuild_remote_rows();
            cx.notify();
            return;
        }
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.error = Some("Remote agent is not connected".into());
            cx.notify();
            return;
        };
        self.tree_loading.insert(path.clone());
        self.rebuild_remote_rows();
        let generation = self.generation;
        let response = WorkspaceBackend::herdr(self.root.clone(), process)
            .list(WorkspacePath::Herdr(path.clone()), self.show_ignored);
        cx.spawn(async move |this, cx| {
            let result = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.tree_loading.remove(&path);
                match result {
                    Ok(listing) => {
                        this.tree_children.insert(path, listing.entries);
                        if listing.truncated {
                            this.error = Some(
                                "This folder has more than 500 entries; showing the first 500."
                                    .into(),
                            );
                        }
                    }
                    Err(error) => this.error = Some(friendly_error(&error)),
                }
                this.rebuild_remote_rows();
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Exactly the native tree's row shell and icon/label rules, with remote
    /// Linux paths kept as Strings. Build rows separately to keep the GPUI
    /// Windows render stack bounded.
    fn render_remote_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.tree_rows.is_empty() {
            return div().flex_1().min_h_0().px_3().py_2()
                .text_sm().text_color(ui::muted())
                .child(if self.loading {
                    "Loading remote folder…"
                } else {
                    "No entries in this folder"
                })
                .into_any_element();
        }
        div().relative().flex_1().min_h_0()
            .child(
                uniform_list(
                    "remote-file-tree-rows",
                    self.tree_rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let nodes = this.tree_rows.clone();
                        nodes[range.clone()].iter().enumerate()
                            .map(|(offset, node)| {
                                this.render_remote_tree_row(node, range.start + offset, cx)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .size_full()
                .p_1p5()
                .track_scroll(&self.tree_scroll),
            )
            .vertical_scrollbar(&self.tree_scroll)
            .into_any_element()
    }

    fn render_remote_tree_row(
        &self, node: &RemoteTreeRow, index: usize, cx: &mut Context<Self>,
    ) -> AnyElement {
        let directory = node.entry.kind == "directory";
        let regular = node.entry.kind == "file";
        let opened = node.expanded;
        let path = node.path.clone();
        let name = node.entry.name.clone();
        let insert_path = node.path.clone();
        file_entry_row(
            SharedString::from(format!("remote-tree-row-{index}")),
            node.depth,
            self.tree_selected_entry.as_ref()
                .is_some_and(|(selected, _)| selected == &node.path),
            &node.entry.name, directory, opened,
            node.entry.ignored, None, None, false,
        )
        .cursor_pointer()
        .when(node.loading, |row| {
            row.child(div().text_xs().text_color(ui::muted()).child("…"))
        })
        .child(
            Button::new(format!("remote-tree-insert-{index}"))
                .ghost().xsmall()
                .icon(Icon::new(IconName::Copy))
                .tooltip("Insert Linux path into agent prompt")
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(RemoteFilesEvent::InsertPath(insert_path.clone()));
                })),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            this.tree_selected_entry = Some((path.clone(), directory));
            this.focus.focus(window, cx);
            this.entry_name.update(cx, |input, cx| {
                input.set_value(name.clone(), window, cx);
            });
            if directory {
                this.tree_selected_directory = Some(path.clone());
                this.toggle_remote_directory(path.clone(), cx);
            } else if regular {
                this.tree_selected_directory = remote_parent(&this.root, &path);
                if event.click_count() >= 2 {
                    cx.emit(RemoteFilesEvent::OpenFile(path.clone()));
                } else {
                    cx.emit(RemoteFilesEvent::PeekFile(path.clone()));
                }
            }
            this.rebuild_remote_rows();
            cx.notify();
        }))
        .into_any_element()
    }


    fn open(
        &mut self, path: String, is_directory: bool,
        window: &mut Window, cx: &mut Context<Self>,
    ) {
        if !self.can_leave_preview(cx) { return; }
        if is_directory {
            self.load_directory(path, cx);
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.preview = None;
        self.editor = None;
        self.markdown = None;
        self._editor_events = None;
        self.error = None;
        self.loading = true;
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.loading = false;
            self.error = Some("Remote agent is not connected".into());
            cx.notify();
            return;
        };
        let response = WorkspaceBackend::herdr(self.root.clone(), process)
            .read(WorkspacePath::Herdr(path.clone()));
        cx.spawn_in(window, async move |this, cx| {
            let response = response.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match response {
                    Ok(preview) => {
                        let is_markdown = this.pane_path.is_some()
                            && std::path::Path::new(&preview.path).extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case("md")
                                    || e.eq_ignore_ascii_case("mdx"));
                        this.markdown = is_markdown.then(|| (
                            MdMode::last(),
                            cx.new(|cx| TextViewState::markdown(&preview.content, cx)),
                        ));
                        this.preview = Some(preview);
                        if this.pane_path.is_some() {
                            this.start_edit(window, cx);
                        }
                    }
                    Err(error) => this.error = Some(friendly_error(&error.to_string())),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Edit only a freshly read file with a server-provided revision.
    fn start_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || self.saving || self.editor.is_some() { return; }
        let Some(preview) = self.preview.as_ref() else { return; };
        if self.root == "/" {
            self.error = Some("Remote editing requires an agent started inside a project directory, not /.".into());
            cx.notify();
            return;
        }
        if preview.revision.is_none() {
            self.error = Some("This agent's bridge supports preview but not editing. Create a new remote agent to edit.".into());
            cx.notify();
            return;
        }
        let content = preview.content.clone();
        // Use the exact EditorState constructor that local FileView uses:
        // syntax highlighting by extension, line numbers and undo.
        let file = std::path::Path::new(&preview.path);
        let editor = cx.new(|cx| super::source_editor(file, content, window, cx));
        editor.update(cx, |input, cx| input.focus(window, cx));
        self._editor_events = Some(cx.subscribe(&editor,
            |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.schedule_markdown_preview(cx);
                    cx.notify();
                }
            }));
        self.editor = Some(editor);
        self.error = None;
        self.apply_pending_draft(window, cx);
        if self.markdown.as_ref().is_some_and(|(mode, _)| *mode == MdMode::Preview) {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// The Markdown view is shared with FileView; only the file bytes are
    /// remote. Debounce re-rendering on edits just as the local editor does.
    fn sync_markdown_preview(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some((_, preview))) = (&self.editor, &self.markdown) else {
            return;
        };
        let text = editor.read(cx).text().to_string();
        preview.update(cx, |view, cx| view.set_text(&text, cx));
    }

    fn schedule_markdown_preview(&mut self, cx: &mut Context<Self>) {
        if self.markdown.is_none() { return; }
        self._markdown_debounce = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(150)).await;
            let _ = this.update(cx, |this, cx| this.sync_markdown_preview(cx));
        });
    }

    fn set_markdown_mode(&mut self, mode: MdMode, window: &mut Window, cx: &mut Context<Self>) {
        let Some((active, _)) = self.markdown.as_mut() else { return; };
        mode.remember();
        *active = mode;
        if mode != MdMode::Edit {
            self.sync_markdown_preview(cx);
        }
        if mode == MdMode::Preview {
            self.focus.focus(window, cx);
        } else if let Some(editor) = &self.editor {
            editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        cx.notify();
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.saving { return; }
        self.pending_draft = None;
        self.restored_revision = None;
        self.editor = None;
        self._editor_events = None;
        self.error = None;
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.loading { return; }
        let (Some(preview), Some(editor)) = (&self.preview, &self.editor) else { return };
        let Some(revision) = self.restored_revision.clone()
            .or_else(|| preview.revision.clone()) else { return };
        let text = editor.read(cx).text().to_string();
        if text.len() > MAX_EDIT_BYTES || text.contains('\0') {
            self.error = Some("Only UTF-8 text up to 2 MiB can be saved.".into());
            cx.notify();
            return;
        }
        if text == preview.content {
            if self.pane_path.is_none() {
                self.cancel_edit(cx);
            }
            return;
        }
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.error = Some("Remote agent disconnected; the draft has not been saved.".into());
            cx.notify();
            return;
        };
        let path = preview.path.clone();
        let generation = self.generation;
        self.saving = true;
        self.error = None;
        // The backend enforces the same revision contract as local files;
        // the Herdr bridge rechecks it before its atomic remote rename.
        let operation = WorkspaceBackend::herdr(self.root.clone(), process)
            .write(WorkspacePath::Herdr(path.clone()), text.clone(), revision);
        // A 128 KiB file takes many 96-byte PTY frames. Run off the GPUI thread.
        let response = cx.background_executor().spawn(operation);
        cx.spawn(async move |this, cx| {
            let result = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                this.saving = false;
                match result {
                    Ok(new_revision) => {
                        this.restored_revision = None;
                        this.preview = Some(RemotePreview {
                            path, content: text, revision: Some(new_revision),
                        });
                        if this.pane_path.is_none() {
                            this.editor = None;
                            this._editor_events = None;
                        }
                        this.error = None;
                    }
                    Err(error) => this.error = Some(friendly_error(&error.to_string())),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    /// Open the same Changes screen for a remote session as for a local one,
    /// without ever touching the Windows worktree.
    /// The architecture-docs screen is a local installer. For a remote
    /// agent, open its own docs folder instead of reading Windows docs.
    pub fn show_remote_docs(&mut self, cx: &mut Context<Self>) {
        self.set_tab(RemoteTab::Files, cx);
        if self.root != "/" {
            let docs = format!("{}/docs", self.root.trim_end_matches('/'));
            self.load_directory(docs, cx);
        }
    }

    pub fn show_changes(&mut self, cx: &mut Context<Self>) {
        self.set_tab(RemoteTab::Changes, cx);
        if self.git.is_none() {
            self.refresh_git(cx);
        }
    }

    /// A branch chosen from the machine-scoped palette opens the remote
    /// Branches view. The actual mutation still requires the existing
    /// second explicit confirmation on the branch row.
    pub fn select_remote_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        self.set_tab(RemoteTab::Branches, cx);
        if self.git.as_ref().is_some_and(|snapshot| {
            snapshot.branches.iter().any(|item| item.local && item.name == branch)
        }) {
            self.git_switch_branch(branch, cx);
        } else {
            self.git_feedback = Some(
                "Refresh remote Branches and select the branch again to confirm.".into(),
            );
            cx.notify();
        }
    }

    fn set_tab(&mut self, tab: RemoteTab, cx: &mut Context<Self>) {
        if self.pane_path.is_some() || self.tab == tab {
            return;
        }
        self.tab = tab;
        if tab != RemoteTab::Files {
            self.refresh_git(cx);
        }
        cx.notify();
    }

    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        self.git_confirm_create = None;
        self.git_confirm_switch = None;
        self.git_confirm_stash = None;
        self.git_confirm_network = None;
        self.git_confirm_worktree = None;
        self.refs.update(cx, |view, cx| {
            view.mark_remote_switch(None, cx);
            view.mark_remote_stash(None, cx);
        });
        self.git_generation = self.git_generation.wrapping_add(1);
        let generation = self.git_generation;
        self.git_loading = true;
        self.git_error = None;
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.git_loading = false;
            self.git_error = Some("Remote agent disconnected. Refresh after reconnection.".into());
            cx.notify();
            return;
        };
        let response = process.request(json!({
            "type": "herdr_git_snapshot", "root": self.root,
        }));
        cx.spawn(async move |this, cx| {
            let result = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.git_generation != generation {
                    return;
                }
                this.git_loading = false;
                match result {
                    Ok(reply) => match parse_git_snapshot(&reply.data) {
                        Ok(snapshot) => {
                            let refs = shared_remote_refs(&snapshot);
                            let tree = shared_remote_worktree(&snapshot);
                            let worktrees = snapshot.worktrees.clone();
                            this.refs.update(cx, |view, cx| {
                                view.set_remote_refs(Some(refs), worktrees, cx);
                            });
                            let writable = snapshot.writes_enabled
                                && !snapshot.files_truncated && this.root != "/";
                            this.changes.update(cx, |view, cx| {
                                view.set_remote_tree(Some(tree), writable, cx);
                            });
                            this.git = Some(snapshot);
                            this.git_error = None;
                        }
                        Err(error) => {
                            this.git = None;
                            this.git_error = Some(error.into());
                        }
                    },
                    Err(error) => {
                        this.git = None;
                        this.git_error = Some(friendly_error(&error.to_string()));
                    }
                }
                if this.git.is_none() {
                    this.refs.update(cx, |view, cx| {
                        view.set_remote_refs(None, Vec::new(), cx);
                    });
                    this.changes.update(cx, |view, cx| {
                        view.set_remote_tree(None, false, cx);
                    });
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    // Stage and Unstage share a typed backend with local Git. The remaining
    // 5D actions retain their guarded bridge calls until migrated.
    fn git_write(&mut self, command: serde_json::Value, cx: &mut Context<Self>) {
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.git_error = Some("Remote agent disconnected; no Git changes were made.".into());
            cx.notify();
            return;
        };
        let committed = command["type"].as_str() == Some("herdr_git_commit");
        let added_worktree = command["type"].as_str() == Some("herdr_git_worktree_add");
        self.git_write_future(async move {
            let reply = process.request(command).await
                .map_err(|error| error.to_string())?;
            let trashed = reply.data["trashToken"].as_str().and_then(|token| {
                reply.data["trashedPath"].as_str()
                    .map(|path| (token.to_owned(), path.to_owned()))
            });
            let worktree = added_worktree.then(|| {
                reply.data["newWorktreePath"].as_str().map(ToOwned::to_owned)
            }).flatten();
            Ok((trashed, committed, worktree))
        }, cx);
    }

    fn git_write_future(
        &mut self, operation: impl std::future::Future<Output = Result<(Option<(String, String)>, bool, Option<String>), String>>
            + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.git_busy || self.git_loading { return; }
        self.git_busy = true;
        self.changes.update(cx, |view, cx| {
            view.set_remote_busy(true, cx);
        });
        self.git_error = None;
        self.git_feedback = None;
        self.git_confirm_create = None;
        self.git_confirm_switch = None;
        self.git_confirm_stash = None;
        self.git_confirm_network = None;
        self.git_confirm_worktree = None;
        self.refs.update(cx, |view, cx| {
            view.mark_remote_switch(None, cx);
            view.mark_remote_stash(None, cx);
        });
        let response = cx.background_executor().spawn(operation);
        cx.spawn(async move |this, cx| {
            let result = response.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.git_busy = false;
                this.changes.update(cx, |view, cx| {
                    view.set_remote_busy(false, cx);
                });
                match result {
                    Ok((trashed, committed, worktree)) => {
                        if let Some(receipt) = trashed {
                            this.last_trash = Some(receipt);
                        }
                        if let Some(path) = worktree {
                            cx.emit(RemoteFilesEvent::OpenWorktreeAgent(path));
                        }
                        if committed {
                            this.changes.update(cx, |view, cx| {
                                view.remote_commit_completed(window, cx);
                            });
                        }
                        this.git_feedback = Some("Remote Git updated. Status refreshed.".into());
                        this.refresh_git(cx);
                    }
                    Err(error) => {
                        let message = friendly_error(&error.to_string());
                        if message.contains("Commit succeeded, but push failed:") {
                            // Commit changed HEAD even though the network push failed.
                            // Refresh before the next mutation; never reuse stale guards.
                            this.changes.update(cx, |view, cx| {
                                view.remote_commit_completed(window, cx);
                            });
                            this.refresh_git(cx);
                            this.git_feedback = Some(message);
                        } else {
                            this.git_error = Some(message);
                        }
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn git_file_action(&mut self, action: GitFileAction, path: String,
        index: char, worktree: char, cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.git.as_ref().filter(|s| s.writes_enabled) else { return; };
        let Some(index_token) = snapshot.index_token.clone() else { return; };
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.git_error = Some("Remote agent disconnected; no Git changes were made.".into());
            cx.notify();
            return;
        };
        let backend = WorkspaceBackend::herdr(self.root.clone(), process);
        let request = GitFileMutation {
            path, source: None, index, worktree,
            herdr_guard: Some(HerdrGitGuard {
                toplevel: snapshot.toplevel.clone(),
                head_oid: snapshot.head_oid.clone(),
                current_branch: snapshot.current_branch.clone(),
                index_token,
            }),
        };
        self.git_write_future(async move {
            backend.git_file(action, request).await.map(|_| (None, false, None))
        }, cx);
    }

    /// Generate on the Linux machine with a bounded staged diff. Never
    /// launch local Windows Git or let a stale response overwrite a new agent.
    fn git_generate_message(&mut self, cx: &mut Context<Self>) {
        if self.git_busy || self.git_loading { return; }
        let Some(snapshot) = self.git.as_ref().filter(|s| {
            s.writes_enabled && !s.files_truncated
                && s.files.iter().any(|entry| entry.staged().is_some())
        }) else { return; };
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.changes.update(cx, |view, cx| {
                view.set_remote_error("Remote agent disconnected.".into(), cx);
            });
            return;
        };
        let request = process.request(json!({
            "type": "herdr_git_generate_message",
            "root": self.root, "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
        }));
        self.changes.update(cx, |view, cx| view.set_remote_generating(true, cx));
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.changes.update(cx, |view, cx| {
                    view.set_remote_generating(false, cx);
                    match result {
                        Ok(reply) => match (reply.data["summary"].as_str(),
                            reply.data["description"].as_str())
                        {
                            (Some(summary), Some(description)) => view.set_remote_message(
                                summary.to_owned(), description.to_owned(), window, cx,
                            ),
                            _ => view.set_remote_error(
                                "Remote generator returned an invalid message.".into(), cx,
                            ),
                        },
                        Err(error) => view.set_remote_error(
                            friendly_error(&error.to_string()), cx,
                        ),
                    }
                });
            });
        }).detach();
    }

    fn git_load_last_commit_message(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|s| s.head_oid.is_some()) else {
            return;
        };
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            return;
        };
        let request = process.request(json!({
            "type": "herdr_git_last_commit",
            "root": self.root,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
        }));
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Ok(reply) = result
                    && let (Some(summary), Some(description)) = (
                        reply.data["summary"].as_str(), reply.data["description"].as_str(),
                    )
                {
                    this.changes.update(cx, |view, cx| {
                        view.set_remote_last_message(
                            summary.to_owned(), description.to_owned(), window, cx,
                        );
                    });
                }
            });
        }).detach();
    }

    /// A confirmed commit from the shared ChangesView; retain Herdr guards.
    /// A bulk operation checks a complete, untruncated, reviewed snapshot
    /// on the bridge before staging only literal file paths.
    fn git_bulk_stage(&mut self, stage: bool, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|snapshot| {
            snapshot.writes_enabled && !snapshot.files_truncated
                && !snapshot.files.is_empty() && snapshot.files.len() <= 80
                && snapshot.index_token.is_some()
        }) else { return; };
        let expected_files: Vec<serde_json::Value> = snapshot.files.iter()
            .map(|entry| json!({
                "path": entry.path,
                "index": entry.index.to_string(),
                "worktree": entry.worktree.to_string(),
            }))
            .collect();
        let command = json!({
            "type": if stage { "herdr_git_stage_all" } else { "herdr_git_unstage_all" },
            "root": self.root,
            "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "expectedFiles": expected_files,
        });
        self.git_write(command, cx);
    }

    fn git_commit_message(
        &mut self, summary: String, description: String, amend: bool, push: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.git.as_ref().filter(|snapshot| {
            snapshot.writes_enabled && !snapshot.files_truncated
                && (snapshot.files.iter().any(|entry| entry.staged().is_some())
                    || (amend && snapshot.head_oid.is_some()))
        }) else { return; };
        if summary.trim().is_empty() || summary.len() > 120
            || summary.chars().any(char::is_control)
            || description.len() > 4000 || description.contains('\0')
        { return; }
        let command = json!({
            "type": "herdr_git_commit", "root": self.root,
            "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "summary": summary, "description": description,
            "amend": amend, "push": push,
        });
        self.git_write(command, cx);
    }

    fn git_create_branch(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|s| s.writes_enabled
            && s.head_oid.is_some() && s.current_branch.is_some())
        else { return; };
        let branch = self.git_branch_name.read(cx).value().trim().to_owned();
        if branch.is_empty() { return; }
        if self.git_confirm_create.as_deref() != Some(branch.as_str()) {
            self.git_confirm_create = Some(branch);
            self.git_feedback = Some("Click Confirm create to add the local branch.".into());
            cx.notify();
            return;
        }
        let command = json!({
            "type": "herdr_git_create_branch", "root": self.root,
            "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid, "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "branch": branch,
        });
        self.git_write(command, cx);
    }

    fn git_discard(&mut self, path: String, index: char, worktree: char,
        staged: bool, cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.git.as_ref().filter(|s|
            s.writes_enabled && s.index_token.is_some()
        ) else { return; };
        if !snapshot.files.iter().any(|entry|
            entry.path == path && entry.index == index && entry.worktree == worktree
        ) { return; }
        // ChangesView already required a native destructive-action dialog.
        // The bridge still validates the exact HEAD/branch/index token and
        // porcelain status before it touches this machine's working tree.
        let Some(absolute) = remote_git_file_path(&snapshot.toplevel, &path) else {
            return;
        };
        let command = json!({
            "type": "herdr_git_discard", "root": self.root,
            "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "path": absolute, "staged": staged,
            "expectedIndex": index.to_string(),
            "expectedWorktree": worktree.to_string(),
        });
        self.git_write(command, cx);
    }

    fn git_worktree_action(
        &mut self, action: RemoteWorktreeAction, branch: Option<String>,
        path: Option<String>, cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.git.as_ref().filter(|s|
            s.writes_enabled && s.head_oid.is_some() && s.index_token.is_some()
        ) else { return; };
        let target = branch.as_deref().or(path.as_deref())
            .unwrap_or("prune").to_owned();
        let operation = (action, target.clone());
        if self.git_confirm_worktree.as_ref() != Some(&operation) {
            self.git_confirm_worktree = Some(operation);
            self.git_feedback = Some(
                "Worktree action requires confirmation; select the same action again.".into(),
            );
            cx.notify();
            return;
        }
        let (kind, new_branch) = match action {
            RemoteWorktreeAction::AddExisting => ("herdr_git_worktree_add", false),
            RemoteWorktreeAction::AddNew => ("herdr_git_worktree_add", true),
            RemoteWorktreeAction::Remove => ("herdr_git_worktree_remove", false),
            RemoteWorktreeAction::Prune => ("herdr_git_worktree_prune", false),
        };
        let command = json!({
            "type": kind,
            "root": self.root, "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "branch": branch, "path": path, "newBranch": new_branch,
        });
        self.git_write(command, cx);
    }

    fn git_network_action(&mut self, action: &'static str, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|s|
            s.writes_enabled && s.head_oid.is_some()
                && s.current_branch.is_some() && s.index_token.is_some()
        ) else { return; };
        if !matches!(action, "fetch" | "pull" | "push") { return; }
        if self.git_confirm_network.as_deref() != Some(action) {
            self.git_confirm_network = Some(action.to_owned());
            self.git_feedback = Some(format!(
                "Confirm {action}: contact the configured Git remote?",
            ));
            cx.notify();
            return;
        }
        let command = json!({
            "type": format!("herdr_git_{action}"),
            "root": self.root, "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
        });
        self.git_write(command, cx);
    }

    fn git_stash_push(&mut self, message: String, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|s|
            s.writes_enabled && !s.files_truncated && !s.files.is_empty()
                && s.head_oid.is_some() && s.index_token.is_some()
        ) else { return; };
        let Some(message) = remote_stash_message(&message) else {
            self.git_error = Some("Stash message must be at most 120 characters on one line.".into());
            cx.notify();
            return;
        };
        let expected_files: Vec<_> = snapshot.files.iter().map(|entry| json!({
            "path": entry.path,
            "index": entry.index.to_string(),
            "worktree": entry.worktree.to_string(),
        })).collect();
        let command = json!({
            "type": "herdr_git_stash_push",
            "root": self.root, "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "expectedFiles": expected_files, "message": message,
        });
        self.git_write(command, cx);
    }

    fn git_stash_action(
        &mut self, name: String, action: StashAction, cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.git.as_ref().filter(|s|
            s.writes_enabled && !s.stashes_truncated && s.index_token.is_some()
                && s.head_oid.is_some()
        ) else { return; };
        let Some(oid) = snapshot.stashes.iter().find(|stash| stash.name == name)
            .and_then(|stash| stash.oid.clone())
        else {
            self.git_error = Some(
                "Stash action requires a new Herdr agent with OID support".into(),
            );
            cx.notify();
            return;
        };
        let pending = (name.clone(), action);
        if self.git_confirm_stash.as_ref() != Some(&pending) {
            self.git_confirm_stash = Some(pending.clone());
            self.refs.update(cx, |view, cx| {
                view.mark_remote_stash(Some(pending), cx);
            });
            self.git_feedback = Some(
                "Review the stash and choose the same action again to confirm.".into(),
            );
            cx.notify();
            return;
        }
        let kind = match action {
            StashAction::Apply => "herdr_git_stash_apply",
            StashAction::Pop => "herdr_git_stash_pop",
            StashAction::Drop => "herdr_git_stash_drop",
        };
        let command = json!({
            "type": kind, "root": self.root,
            "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid,
            "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "stash": name, "expectedStashOid": oid,
        });
        self.git_write(command, cx);
    }

    fn git_switch_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|s| s.writes_enabled
            && s.head_oid.is_some() && s.current_branch.is_some())
        else { return; };
        if self.git_confirm_switch.as_deref() != Some(branch.as_str()) {
            self.git_confirm_switch = Some(branch.clone());
            self.refs.update(cx, |view, cx| {
                view.mark_remote_switch(Some(branch), cx);
            });
            self.git_feedback = Some(
                "Branch switching requires a clean tree. Click the same branch again to confirm."
                    .into());
            cx.notify();
            return;
        }
        let command = json!({
            "type": "herdr_git_switch_branch", "root": self.root,
            "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid, "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "branch": branch,
        });
        self.git_write(command, cx);
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> Div {
        let count = self.git.as_ref().map_or(0, |snapshot| snapshot.files.len());
        h_flex()
            .flex_none()
            .px_2()
            .pb_1p5()
            .gap_1()
            .child(super::tabs::tab_button(RemoteTab::Files, self.tab, 0)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_tab(RemoteTab::Files, cx)
                })))
            .child(super::tabs::tab_button(RemoteTab::Changes, self.tab, count)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_tab(RemoteTab::Changes, cx)
                })))
            .child(super::tabs::tab_button(RemoteTab::Branches, self.tab, 0)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_tab(RemoteTab::Branches, cx)
                })))
    }

    /// Build each row in its own stack frame and erase the resulting
    /// GPUI element type before returning to the list builder.
    /// Both native and Herdr use the same checkbox row. The backend
    /// provides the operation and guards; there is no host-specific UI.
    fn push_git_create_branch_form(
        &self, rows: &mut Vec<AnyElement>, snapshot: &RemoteGitSnapshot,
        cx: &mut Context<Self>,
    ) {
        let name = self.git_branch_name.read(cx).value().trim().to_owned();
        let confirmed = self.git_confirm_create.as_deref() == Some(name.as_str());
        rows.push(
            v_flex().px_3().py_3().gap_2().border_t_1()
                .border_color(ui::border())
                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD)
                    .child("Create local branch from HEAD"))
                .child(Input::new(&self.git_branch_name).small())
                .child(
                    Button::new("remote-git-create-branch")
                        .ghost().small().w_full()
                        .label(if confirmed { "Confirm create" } else { "Create branch" })
                        .disabled(self.git_busy || self.git_loading || name.is_empty()
                            || snapshot.head_oid.is_none()
                            || snapshot.current_branch.is_none())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.git_create_branch(cx);
                        })),
                )
                .child(div().text_xs().text_color(ui::muted())
                    .child("Switch refuses staged, unstaged and untracked changes."))
                .into_any_element(),
        );
    }

    /// The list owns only type-erased elements, avoiding a huge combined
    /// GPUI render expression and keeping 5C/5D behavior identical.
    fn render_branch_create(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(snapshot) = self.git.as_ref().filter(|snapshot| {
            snapshot.writes_enabled && self.root != "/"
        }) {
            self.push_git_create_branch_form(&mut rows, snapshot, cx);
        }
        v_flex().children(rows).into_any_element()
    }

    /// Keep the root GPUI render shallow on Windows debug builds.
    /// 5D previously built every interactive row and both editors inside
    /// this one render frame, overflowing the Windows UI thread stack.
    fn render_git_panel(&self, cx: &mut Context<Self>) -> Div {
        panel_shell()
            .child(
                panel_header()
                    .child(Icon::new(IconName::GitBranch).small().text_color(ui::petal()))
                    .child(div().flex_1().min_w_0().truncate().text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Remote Git · {}", self.machine)))
                    .child(
                        Button::new("remote-git-refresh")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::RefreshCw))
                            .disabled(self.git_busy || self.git_loading)
                            .tooltip("Refresh remote Git status and branches")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_git(cx))),
                    )
                    .child(
                        Button::new("remote-git-close").ghost().xsmall()
                            .icon(Icon::new(IconName::X))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(RemoteFilesEvent::Close);
                            })),
                    ),
            )
            .child(self.render_tabs(cx))
            .child(
                h_flex().px_2().py_1().gap_1().flex_none()
                .child(
                        Button::new("remote-git-fetch")
                            .ghost().xsmall().label(if self.git_confirm_network.as_deref()
                                == Some("fetch") { "Confirm Fetch" } else { "Fetch" })
                            .disabled(self.git_busy || self.git_loading ||
                                !self.git.as_ref().is_some_and(|s| s.writes_enabled))
                            .tooltip("Fetch refs from a safe configured remote")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_network_action("fetch", cx)
                            })),
                    )
                .child(
                        Button::new("remote-git-pull")
                            .ghost().xsmall().label(if self.git_confirm_network.as_deref()
                                == Some("pull") { "Confirm Pull" } else { "Pull" })
                            .disabled(self.git_busy || self.git_loading ||
                                !self.git.as_ref().is_some_and(|s| s.writes_enabled))
                            .tooltip("Fast-forward only; refuses a dirty tree")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_network_action("pull", cx)
                            })),
                    )
                .child(
                        Button::new("remote-git-push")
                            .ghost().xsmall().label(if self.git_confirm_network.as_deref()
                                == Some("push") { "Confirm Push" } else { "Push" })
                            .disabled(self.git_busy || self.git_loading ||
                                !self.git.as_ref().is_some_and(|s| s.writes_enabled))
                            .tooltip("Non-force push of only this branch")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_network_action("push", cx)
                            })),
                    )
            )
            .when(self.git_loading, |panel| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::muted())
                    .child("Reading status from remote Git…"),
            ))
            .when(self.git_busy, |panel| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::muted())
                    .child("Applying guarded remote Git operation…"),
            ))
            .when_some(self.git_error.clone(), |panel, error| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::warning())
                    .child(error),
            ))
            .when_some(self.git_feedback.clone(), |panel, feedback| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::muted())
                    .child(feedback),
            ))
            .when(
                self.tab == RemoteTab::Branches
                    && self.git.as_ref().is_some_and(|snapshot| {
                        snapshot.branches_truncated || snapshot.tags_truncated
                    }),
                |panel| {
                    panel.child(
                        div().px_3().py_2().text_xs().text_color(ui::warning())
                            .child("Remote branches or tags were truncated; this reference list is incomplete."),
                    )
                },
            )
            .when(self.tab == RemoteTab::Changes, |panel| {
                panel.child(
                    div().flex_1().min_h_0().child(self.changes.clone()),
                )
            })
            .when(self.tab == RemoteTab::Branches, |panel| {
                panel
                    .child(div().flex_1().min_h_0().child(self.refs.clone()))
                    .child(self.render_branch_create(cx))
            })
            .child(
                div().h(px(26.)).flex_none().border_t_1()
                    .border_color(ui::border()).px_3().text_size(px(10.))
                    .text_color(ui::muted())
                    .child(if self.git.as_ref().is_some_and(|s| s.writes_enabled) {
                        "Remote Git · guarded operations · confirmation required"
                    } else {
                        "Remote Git · read-only · open a new 5D agent for writes"
                    }),
            )
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.tab == RemoteTab::Files {
            self.load_directory(self.current.clone(), cx);
        } else {
            self.refresh_git(cx);
        }
    }
}

impl Focusable for RemoteFilesView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.as_ref().map_or_else(
            || self.focus.clone(), |editor| editor.focus_handle(cx)
        )
    }
}

impl Render for RemoteFilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pane_path.is_none() && self.tab != RemoteTab::Files {
            return self.render_git_panel(cx).into_any_element();
        }
        let parent = remote_parent(&self.root, &self.current);
        let central = self.pane_path.is_some();
        // This must be a direct flex child of panel_shell. An extra plain
        // div wrapper makes the inner uniform_list resolve a zero-height
        // viewport on some GPUI/Windows layouts, even with loaded rows.
        let mut content = v_flex()
            .gap(px(1.))
            .flex_1()
            .min_h_0()
            .overflow_hidden();
        if let Some(preview) = &self.preview {
            let copied = preview.path.clone();
            let rows: Vec<String> = preview.content.lines().take(600).map(str::to_owned).collect();
            let truncated = preview.content.lines().count() > rows.len();
             let can_edit = preview.revision.is_some() && self.root != "/";
            let dirty = self.editor.as_ref().is_some_and(|editor| {
                *editor.read(cx).text() != preview.content
            });
            content = content.child(
                h_flex()
                    .px_2()
                    .py_2()
                    .gap_1()
                    .when(!central, |header| header.child(
                        Button::new("remote-files-back-preview")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::ChevronLeft))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_directory(this.current.clone(), cx)
                            })),
                    ))
                    .child(div().flex_1().min_w_0().truncate()
                        .text_xs().child(preview.path.clone()))
                    .child(
                        Button::new("remote-files-insert-preview")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::Copy))
                            .tooltip("Insert remote Linux path in prompt")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(RemoteFilesEvent::InsertPath(copied.clone()));
                            })),
                    )
            );
            if let Some(editor) = &self.editor {
                content = content.child(
                    h_flex().px_2().gap_2().items_center()
                        .child(
                            Button::new("remote-files-save")
                                .primary().small().label(if self.saving {
                                    "Saving…".to_owned()
                                } else {
                                    format!("Save · {SAVE_HINT}")
                                })
                                .disabled(!dirty || self.saving)
                                .on_click(cx.listener(|this, _, _, cx| this.save_edit(cx))),
                        )
                        .when(!central, |row| row.child(
                            Button::new("remote-files-cancel-edit")
                                .ghost().small().label("Cancel")
                                .disabled(self.saving)
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_edit(cx))),
                        ))
                        .child(div().flex_1())
                        .child(div().text_xs().text_color(ui::muted())
                            .child(if dirty { "Unsaved changes" } else { "Unchanged" }))
                );
                let editor_view = || {
                    Editor::new(editor)
                        .appearance(false)
                        .disabled(self.saving)
                        .font_family(ui::MONO)
                        .text_size(px(12.5))
                        .size_full()
                };
                let render_markdown = |preview: &Entity<TextViewState>| {
                    div()
                        .size_full().overflow_hidden()
                        .text_size(px(14.))
                        .child(
                            TextView::new(preview)
                                .scrollable(true)
                                .selectable(true)
                                .stream_fade(false)
                                .px_6().py_4()
                        )
                };
                let body: AnyElement = match &self.markdown {
                    Some((MdMode::Preview, md)) if central =>
                        render_markdown(md).into_any_element(),
                    Some((MdMode::Split, md)) if central =>
                        h_flex()
                            .size_full().gap_2()
                            .child(div().flex_1().min_w_0().h_full()
                                .child(editor_view()))
                            .child(div().flex_1().min_w_0().h_full()
                                .child(render_markdown(md)))
                            .into_any_element(),
                    _ => editor_view().into_any_element(),
                };
                content = content.child(
                    div().px_2().py_2()
                        .h(px(if central {
                            (f32::from(window.viewport_size().height) - 162.).max(260.)
                        } else { 360. }))
                        .w_full().overflow_hidden()
                        .rounded_md().border_1().border_color(ui::border())
                        .child(body)
                );
            } else {
                content = content
                    .child(
                        h_flex().px_2().py_1().items_center()
                            .child(
                                Button::new("remote-files-edit")
                                    .ghost().small().label("Edit")
                                    .disabled(!can_edit || self.loading)
                                    .tooltip(if can_edit {
                                        "Edit this existing remote text file"
                                    } else {
                                        "Read-only: use a 5B agent in a project directory"
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.start_edit(window, cx)
                                    })),
                            )
                            .child(div().flex_1())
                            .child(div().text_xs().text_color(ui::muted())
                                .child("UTF-8 text · 2 MiB maximum; binary via Download")),
                    )
                    .children(rows.into_iter().enumerate().map(|(i, text)| {
                        h_flex()
                            .id(("remote-preview-line", i))
                            .gap_2().px_2()
                            .font_family(ui::MONO)
                            .text_size(px(11.))
                            .child(div().w(px(30.)).text_color(ui::faint())
                                .child((i + 1).to_string()))
                            .child(div().flex_1().min_w_0().text_color(ui::fg()).child(text))
                    }))
                    .when(truncated, |section| section.child(
                        div().px_2().py_2().text_xs().text_color(ui::warning())
                            .child("Preview limited to the first 600 lines"),
                    ));
            }
        } else {
            if let Some(parent) = parent {
                content = content.child(
                    h_flex().id("remote-files-parent")
                        .h(px(26.)).px_2().gap(px(5.))
                        .cursor_pointer()
                        .hover(|row| row.bg(ui::surface_hover()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.load_directory(parent.clone(), cx)
                        }))
                        .child(Icon::new(IconName::ChevronLeft).small().text_color(ui::muted()))
                        .child(div().text_xs().child("..")),
                );
            }
            content = content.child(self.render_remote_tree(cx));
        }

        panel_shell()
            .key_context(if central { CONTEXT } else { TREE_CONTEXT })
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::save_shortcut))
            .on_action(cx.listener(Self::tree_up))
            .on_action(cx.listener(Self::tree_down))
            .on_action(cx.listener(Self::tree_left))
            .on_action(cx.listener(Self::tree_right))
            .on_action(cx.listener(Self::tree_open))
            .on_action(cx.listener(Self::tree_rename))
            .on_action(cx.listener(Self::tree_copy))
            .on_action(cx.listener(Self::tree_cut))
            .on_action(cx.listener(Self::tree_paste))
            .child(
                panel_header()
                    .child(Icon::new(if central { IconName::FileCode }
                        else { IconName::Layers })
                        .small().text_color(ui::petal()))
                    .child(div().flex_1().min_w_0().truncate()
                        .text_sm().font_weight(FontWeight::SEMIBOLD)
                        .child(if let Some(path) = &self.pane_path {
                            format!("{} · {}", self.machine, path)
                        } else {
                            if super::backend::remote_fs_trace_enabled() {
                                format!(
                                    "Remote Files · {} · {} rows",
                                    self.machine,
                                    self.tree_rows.len(),
                                )
                            } else {
                                format!("Remote Files · {}", self.machine)
                            }
                        }))
                    .when(!central, |bar| bar.child(
                        Button::new("remote-show-ignored")
                            .ghost().xsmall()
                            .icon(Icon::new(if self.show_ignored {
                                IconName::Eye
                            } else {
                                IconName::EyeOff
                            }))
                            .tooltip(if self.show_ignored {
                                "Hide ignored and hidden files"
                            } else {
                                "Show ignored and hidden files"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_ignored(cx)
                            })),
                    ))
                    .when_some(if central {
                        self.markdown.as_ref().map(|(mode, _)| *mode)
                    } else { None }, |bar, mode| {
                        let weak = cx.entity().downgrade();
                        bar.child(ui::segmented(
                            "remote-file-mode", &MdMode::LABELS, Some(mode.index()),
                            move |ix, window, cx| {
                                if let Some(&selected) = MdMode::ALL.get(*ix) {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.set_markdown_mode(selected, window, cx)
                                    });
                                }
                            },
                        ))
                    })
                    .when(!central, |bar| bar.child(
                        Button::new("remote-files-refresh")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::RefreshCw))
                            .tooltip("Refresh this remote directory")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ))
                    .when(!central, |bar| bar.child(
                        Button::new("remote-files-close")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::X))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.can_leave_preview(cx) {
                                    cx.emit(RemoteFilesEvent::Close);
                                }
                            })),
                    )),
            )
            .when(!central, |panel| panel.child(self.render_tabs(cx)))
            .when(!central, |panel| panel.child(
                h_flex().px_2().pb_1().gap_1()
                    .child(Button::new("remote-tree-select-root")
                        .ghost().xsmall().label("Root")
                        .tooltip("Create items in the workspace root")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tree_selected_directory = None;
                            this.tree_selected_entry = None;
                            cx.notify();
                        })))
                    .child(div().flex_1().min_w_0()
                        .child(Input::new(&self.create_name).small()))
                    .child(Button::new("remote-create-file")
                        .ghost().xsmall()
                        .icon(Icon::new(IconName::FilePlus))
                        .tooltip("New file in selected folder (or workspace root)")
                        .disabled(self.creating || self.entry_busy || self.root == "/")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_remote_entry(false, window, cx)
                        })))
                    .child(Button::new("remote-create-folder")
                        .ghost().xsmall()
                        .icon(Icon::new(IconName::FolderPlus))
                        .tooltip("New folder in selected folder (or workspace root)")
                        .disabled(self.creating || self.entry_busy || self.root == "/")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_remote_entry(true, window, cx)
                        })))
                    .child(Button::new("remote-transfer-upload")
                        .ghost().xsmall().label("Upload")
                        .tooltip("Upload a local file (binary-safe, 16 MiB maximum)")
                        .disabled(self.transfer_busy || self.root == "/")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.upload_local_file(window, cx)
                        })))
                    .child(Button::new("remote-transfer-download")
                        .ghost().xsmall().label("Download")
                        .tooltip("Download selected remote file to a local folder")
                        .disabled(self.transfer_busy || self.tree_selected_entry.as_ref()
                            .is_none_or(|(_, directory)| *directory))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.download_remote_file(window, cx)
                        }))),
            ))
            .when(!central, |panel| panel.child(
                v_flex().px_2().pb_1().gap_1()
                    .child(
                        h_flex().gap_1()
                            .child(div().flex_1().min_w_0()
                                .child(Input::new(&self.entry_name).small()))
                            .child(Button::new("remote-entry-rename")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::Pencil))
                                .tooltip("Rename selected remote file or directory")
                                .disabled(self.entry_busy
                                    || self.tree_selected_entry.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.rename_remote_entry(cx);
                                })))
                            .child(Button::new("remote-entry-duplicate")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::CopyPlus))
                                .tooltip("Duplicate selected file or folder (bounded copy)")
                                .disabled(self.entry_busy
                                    || self.tree_selected_entry.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.run_remote_entry(
                                        WorkspaceEntryAction::Duplicate, cx,
                                    );
                                }))),
                    )
                    .child(
                        h_flex().gap_1()
                            .child(Button::new("remote-entry-cut")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::Scissors))
                                .tooltip("Cut selected remote file or directory")
                                .disabled(self.entry_busy
                                    || self.tree_selected_entry.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clipboard_remote_entry(true, cx)
                                })))
                            .child(Button::new("remote-entry-copy")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::Copy))
                                .tooltip("Copy selected file or folder")
                                .disabled(self.entry_busy
                                    || self.tree_selected_entry.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clipboard_remote_entry(false, cx)
                                })))
                            .child(Button::new("remote-entry-paste")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::ClipboardPaste))
                                .tooltip("Paste into selected folder, or workspace root")
                                .disabled(self.entry_busy || self.file_clipboard.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.paste_remote_entry(cx)
                                })))
                            .child(Button::new("remote-entry-trash")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::Trash))
                                .label(if self.trash_confirm.as_ref()
                                    == self.tree_selected_entry.as_ref().map(|(path, _)| path)
                                    && self.trash_confirm.is_some()
                                {
                                    "Confirm Trash"
                                } else {
                                    "Trash"
                                })
                                .tooltip("Move to recoverable private Trash; click twice to confirm")
                                .disabled(self.entry_busy || self.tree_selected_entry.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.trash_remote_entry(cx)
                                })))
                            .when(self.last_trash.is_some(), |row| row.child(
                                Button::new("remote-entry-restore")
                                    .ghost().xsmall()
                                    .icon(Icon::new(IconName::Undo2))
                                    .label("Undo")
                                    .tooltip("Restore last entry from private Trash")
                                    .disabled(self.entry_busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.restore_remote_entry(cx)
                                    })),
                            ))
                            .child(div().flex_1())
                            .child(div().min_w_0().truncate().text_xs()
                                .text_color(ui::muted())
                                .child(self.tree_selected_directory.as_ref()
                                    .unwrap_or(&self.root).clone())),
                    ),
            ))
            .when(!central && !self.transfer_targets.is_empty(), |panel| {
                panel.child(
                    h_flex().px_2().pb_1().gap_1().flex_wrap().children(
                        self.transfer_targets.iter().enumerate().map(|(index, (label, address))| {
                            let address = address.clone();
                            Button::new(("remote-relay", index))
                                .ghost().xsmall().label(format!("Send to {label}"))
                                .tooltip("Copy selected remote file to another live agent; never overwrite")
                                .disabled(self.transfer_busy || self.tree_selected_entry.as_ref()
                                    .is_none_or(|(_, directory)| *directory))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.relay_selected_file(address.clone(), cx)
                                }))
                        })
                    ),
                )
            })
            .when(self.transfer_busy, |panel| panel.child(
                div().px_3().py_1().text_xs().text_color(ui::muted())
                    .child("Transferring file over Herdr…"),
            ))
            .when_some(self.transfer_feedback.clone(), |panel, feedback| panel.child(
                div().px_3().py_1().text_xs().text_color(ui::muted())
                    .child(feedback),
            ))
            .when(self.loading, |panel| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::muted())
                    .child("Reading from remote agent…"),
            ))
            .when_some(self.error.clone(), |panel, error| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::warning())
                    .child(error),
            ))
            // Preserve flex sizing all the way into the lazy tree. The
            // previous unstyled div wrapper had no flex layout and could
            // collapse the list's viewport despite successful RPC results.
            .child(content)
            .when(self.truncated, |panel| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::muted())
                    .child("Showing first 500 entries"),
            ))
            .child(
                div().h(px(26.)).flex_none()
                    .border_t_1().border_color(ui::border())
                    .px_3().text_size(px(10.))
                    .text_color(ui::muted())
                    .child(if self.editor.is_some() {
                        "Editing · SHA-256 conflict protection · 2 MiB"
                    } else {
                        "Files · edit ≤2 MiB · transfer ≤16 MiB"
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_git_snapshot, remote_child, remote_parent, retryable_remote_loading_error,
    };

    #[test]
    fn remote_listing_retries_only_transport_failures() {
        assert!(retryable_remote_loading_error(
            "Waiting for this remote agent to connect…"
        ));
        assert!(retryable_remote_loading_error(
            "Herdr RPC bridge is not connected"
        ));
        assert!(!retryable_remote_loading_error(
            "The requested root does not match this agent's working directory"
        ));
        assert!(!retryable_remote_loading_error(
            "Permission denied"
        ));
    }

    #[test]
    fn remote_paths_remain_under_root() {
        assert_eq!(remote_child("/srv/workspaces/demo", "Cargo.toml"),
            "/srv/workspaces/demo/Cargo.toml");
        assert_eq!(remote_child("/", "srv"), "/srv");
        assert_eq!(remote_parent("/srv/workspaces/demo", "/srv/workspaces/demo/src"),
            Some("/srv/workspaces/demo".to_owned()));
        assert_eq!(remote_parent("/srv/workspaces/demo", "/srv/workspaces/demo"), None);
    }
    #[test]
    fn git_snapshot_accepts_status_and_branches_from_selected_machine() {
        let data = serde_json::json!({
            "toplevel": "/srv/workspaces/JaviPi",
            "readOnly": true,
            "files": [
                { "path": "src/main.rs", "index": "M", "worktree": " " },
                { "path": "README.md", "index": " ", "worktree": "M" }
            ],
            "branches": [
                { "name": "main", "current": true },
                { "name": "feature/rust", "current": false }
            ],
            "filesTruncated": false,
            "branchesTruncated": false
        });
        let snapshot = parse_git_snapshot(&data).unwrap();
        assert_eq!(snapshot.files.len(), 2);
        assert_eq!(snapshot.files[0].index, 'M');
        assert_eq!(snapshot.files[1].worktree, 'M');
        assert!(snapshot.branches[0].current);
    }

    #[test]
    fn git_snapshot_rejects_local_and_parent_paths() {
        let base = serde_json::json!({
            "toplevel": "/srv/workspaces/JaviPi",
            "readOnly": true,
            "files": [{"path": "../secret", "index": "?", "worktree": "?"}],
            "branches": []
        });
        assert!(parse_git_snapshot(&base).is_err());
        for bad in ["C:/Users/demo", "/tmp/x"] {
            let mut data = base.clone();
            data["files"][0]["path"] = serde_json::json!(bad);
            assert!(parse_git_snapshot(&data).is_err());
        }
        let mut writable = base;
        writable["files"] = serde_json::json!([]);
        writable["readOnly"] = serde_json::json!(false);
        assert!(parse_git_snapshot(&writable).is_err());
    }

}
