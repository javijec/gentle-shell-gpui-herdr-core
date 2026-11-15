//! Linux Files bound to one Herdr Pi agent. Remote preview and opt-in
//! revision-checked UTF-8 editing run through the existing injected bridge.
//! This view never uses local Windows filesystem APIs or remote Pi bash.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use crate::features::session::SessionView;
use super::backend::{
    GitFileAction, GitFileMutation, HerdrGitGuard, WorkspaceBackend,
    WorkspaceDirEntry as RemoteEntry, WorkspacePath,
    WorkspaceText as RemotePreview,
};
use gentle_core::git::status::{EntryKind, StatusEntry as RemoteGitFile};
use super::changes_ui::{ChangeSide, change_row, section_header, side_entries};
use super::tabs::FilePanelTab as RemoteTab;
use super::viewer::MdMode;
use crate::shared::ui;

pub enum RemoteFilesEvent {
    InsertPath(String),
    /// Open an existing remote Linux file in a normal central editor pane.
    OpenFile(String),
    /// Request a native central diff for a changed Linux file.
    OpenDiff(String),
    Close,
}

struct RemoteGitBranch {
    name: String,
    current: bool,
    local: bool,
}

struct RemoteGitSnapshot {
    toplevel: String,
    head_oid: Option<String>,
    current_branch: Option<String>,
    index_token: Option<String>,
    writes_enabled: bool,
    files: Vec<RemoteGitFile>,
    branches: Vec<RemoteGitBranch>,
    files_truncated: bool,
    branches_truncated: bool,
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
        files, branches,
        files_truncated: data["filesTruncated"].as_bool().unwrap_or(false),
        branches_truncated: data["branchesTruncated"].as_bool().unwrap_or(false),
    })
}

#[cfg(test)]
mod remote_git_snapshot_tests {
    use super::parse_git_snapshot;
    use serde_json::json;

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

const MAX_EDIT_BYTES: usize = 128 * 1024;
const CONTEXT: &str = "GentleRemoteFileView";
const SAVE_HINT: &str = if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" };

actions!(herdr_file_view, [Save]);

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
    git_summary: Entity<InputState>,
    git_branch_name: Entity<InputState>,
    git_confirm_commit: Option<String>,
    git_confirm_create: Option<String>,
    git_confirm_switch: Option<String>,
    _git_events: Vec<Subscription>,
    focus: FocusHandle,
    entries: Vec<RemoteEntry>,
    preview: Option<RemotePreview>,
    editor: Option<Entity<EditorState>>,
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
        cx.bind_keys([KeyBinding::new(key, Save, Some(CONTEXT))]);
    }

    fn save_shortcut(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save_edit(cx);
    }

    pub fn new(
        session: Entity<SessionView>,
        machine: String,
        root: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let git_summary = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Commit summary (staged files)")
        });
        let git_branch_name = cx.new(|cx| {
            InputState::new(window, cx).placeholder("New local branch name")
        });
        let git_events = vec![
            cx.subscribe(&git_summary, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event { cx.notify(); }
            }),
            cx.subscribe(&git_branch_name, |_, _, event: &InputEvent, cx| {
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
            git_summary,
            git_branch_name,
            git_confirm_commit: None,
            git_confirm_create: None,
            git_confirm_switch: None,
            _git_events: git_events,
            focus: cx.focus_handle(),
            entries: Vec::new(),
            preview: None,
            editor: None,
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
        if view.root.is_empty() {
            view.error = Some(
                "This restored agent has no known remote working directory yet. Refresh after Herdr discovers its terminal, or open a new agent."
                    .into(),
            );
            return view;
        }
        view.load_directory(view.root.clone(), cx);
        // A newly launched remote Pi may still be connecting when the
        // right panel appears. Wait for the SAME agent to become ready.
        if view.error.is_some() {
            view._wait_for_agent = cx.spawn_in(window, async move |this, cx| {
                for _ in 0..45 {
                    cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                    let ready = this.update_in(cx, |this, window, cx| {
                        if this.session.read(cx).remote_fs_process().is_some() {
                            if let Some(path) = this.pane_path.clone() {
                                this.open(path, false, window, cx);
                            } else {
                                this.load_directory(this.root.clone(), cx);
                            }
                            true
                        } else {
                            false
                        }
                    });
                    match ready {
                        Ok(true) | Err(_) => break,
                        Ok(false) => {}
                    }
                }
            });
        }
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

    pub fn file_title(&self, cx: &App) -> SharedString {
        let Some(path) = self.pane_path.as_ref() else {
            return "Remote Files".into();
        };
        let filename = path.rsplit('/').next().unwrap_or(path);
        let dirty = if self.has_unsaved(cx) { " •" } else { "" };
        format!("{filename}{dirty} · {}", self.machine).into()
    }

    pub fn has_unsaved(&self, cx: &App) -> bool {
        self.saving || self.editor.as_ref().zip(self.preview.as_ref()).is_some_and(
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
        self.editor.is_some() || self.saving
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
        self.current = directory.clone();
        self.preview = None;
        self.editor = None;
        self.markdown = None;
        self._editor_events = None;
        self.entries.clear();
        self.error = None;
        self.truncated = false;
        self.loading = true;
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.loading = false;
            self.error = Some("Waiting for this remote agent to connect…".into());
            cx.notify();
            return;
        };
        let response = WorkspaceBackend::herdr(self.root.clone(), process)
            .list(WorkspacePath::Herdr(directory), false);
        cx.spawn(async move |this, cx| {
            let response = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match response {
                    Ok(listing) => {
                        this.entries = listing.entries;
                        this.truncated = listing.truncated;
                    }
                    Err(error) => this.error = Some(friendly_error(&error.to_string())),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
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
        if self.markdown.as_ref().is_some_and(|(mode, _)| *mode == MdMode::Preview) {
            self.focus.focus(window, cx);
        }
        self.error = None;
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
        self.editor = None;
        self._editor_events = None;
        self.error = None;
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.loading { return; }
        let (Some(preview), Some(editor)) = (&self.preview, &self.editor) else { return };
        let Some(revision) = preview.revision.clone() else { return };
        let text = editor.read(cx).text().to_string();
        if text.len() > MAX_EDIT_BYTES || text.contains('\0') {
            self.error = Some("Only UTF-8 text up to 128 KiB can be saved.".into());
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
        self.git_confirm_commit = None;
        self.git_confirm_create = None;
        self.git_confirm_switch = None;
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
        self.git_write_future(async move {
            process.request(command).await.map(|_| ())
                .map_err(|error| error.to_string())
        }, cx);
    }

    fn git_write_future(
        &mut self, operation: impl std::future::Future<Output = Result<(), String>>
            + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.git_busy || self.git_loading { return; }
        self.git_busy = true;
        self.git_error = None;
        self.git_feedback = None;
        self.git_confirm_commit = None;
        self.git_confirm_create = None;
        self.git_confirm_switch = None;
        let response = cx.background_executor().spawn(operation);
        cx.spawn(async move |this, cx| {
            let result = response.await;
            let _ = this.update(cx, |this, cx| {
                this.git_busy = false;
                match result {
                    Ok(_) => {
                        this.git_feedback = Some("Remote Git updated. Status refreshed.".into());
                        this.refresh_git(cx);
                    }
                    Err(error) => {
                        this.git_error = Some(friendly_error(&error.to_string()));
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
        self.git_write_future(backend.git_file(action, request), cx);
    }

    fn git_commit(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|s| s.writes_enabled
            && !s.files_truncated && s.files.iter()
                .any(|entry| entry.staged().is_some()))
        else { return; };
        let summary = self.git_summary.read(cx).value().trim().to_owned();
        if summary.is_empty() { return; }
        if self.git_confirm_commit.as_deref() != Some(summary.as_str()) {
            self.git_confirm_commit = Some(summary);
            self.git_feedback = Some(
                "Review the staged changes, then click Confirm commit.".into());
            cx.notify();
            return;
        }
        let command = json!({
            "type": "herdr_git_commit", "root": self.root,
            "toplevel": snapshot.toplevel,
            "expectedHead": snapshot.head_oid, "expectedBranch": snapshot.current_branch,
            "expectedIndexToken": snapshot.index_token,
            "summary": summary,
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

    fn git_switch_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        let Some(snapshot) = self.git.as_ref().filter(|s| s.writes_enabled
            && s.head_oid.is_some() && s.current_branch.is_some())
        else { return; };
        if self.git_confirm_switch.as_deref() != Some(branch.as_str()) {
            self.git_confirm_switch = Some(branch);
            self.git_feedback = Some(
                "Branch switching requires a clean tree. Click Confirm switch.".into());
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
    fn push_git_file_row(
        &self, rows: &mut Vec<AnyElement>, snapshot: &RemoteGitSnapshot,
        index: usize, entry: &RemoteGitFile, side: ChangeSide, badge: char,
        writable: bool, cx: &mut Context<Self>,
    ) {
        let diff_target = format!(
            "{}/{}", snapshot.toplevel.trim_end_matches('/'), entry.path
        );
        let action = match side {
            ChangeSide::Unstaged => GitFileAction::Stage,
            ChangeSide::Staged => GitFileAction::Unstage,
        };
        let stage_target = entry.path.clone();
        let index_state = entry.index;
        let worktree_state = entry.worktree;
        let allowed = writable && entry.kind != EntryKind::Conflict
            && !self.git_busy && !self.git_loading;
        let id = format!("remote-change-{side:?}-{index}");
        let weak = cx.entity().downgrade();
        let control = div()
            .id(SharedString::from(format!("{id}-check-wrap")))
            .flex_none()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(Checkbox::new(SharedString::from(format!("{id}-check")))
                .small()
                .checked(side == ChangeSide::Staged)
                .disabled(!allowed)
                .on_click(move |_, _, cx| {
                    let target = stage_target.clone();
                    let _ = weak.update(cx, |this, cx| {
                        this.git_file_action(action, target, index_state, worktree_state, cx);
                    });
                }));
        rows.push(
            change_row(
                SharedString::from(id), &entry.path, badge, false, 0., control,
            )
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(RemoteFilesEvent::OpenDiff(diff_target.clone()));
                }))
                .into_any_element(),
        );
    }

    fn push_git_section_header(
        &self, rows: &mut Vec<AnyElement>, side: ChangeSide, count: usize,
    ) {
        // Bulk writes must be a separate guarded server operation. Their
        // controls remain visible but disabled until that protocol exists.
        let checkbox = Checkbox::new(SharedString::from(
            format!("remote-{side:?}-all-check"),
        ))
            .small()
            .checked(side == ChangeSide::Staged)
            .disabled(true);
        let button = Button::new(format!("remote-{side:?}-all"))
            .outline()
            .xsmall()
            .label(side.action_label())
            .disabled(true)
            .tooltip("Bulk Stage/Unstage is not supported by this Herdr bridge");
        rows.push(section_header(side, count, checkbox, button).into_any_element());
    }

    fn push_git_commit_form(
        &self, rows: &mut Vec<AnyElement>, snapshot: &RemoteGitSnapshot,
        cx: &mut Context<Self>,
    ) {
        let staged_count = snapshot.files.iter()
            .filter(|entry| entry.staged().is_some()).count();
        let summary = self.git_summary.read(cx).value().trim().to_owned();
        let confirmed = self.git_confirm_commit.as_deref() == Some(summary.as_str());
        rows.push(
            v_flex().px_3().py_3().gap_2().border_t_1()
                .border_color(ui::border())
                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Commit staged changes ({staged_count} visible)")))
                .child(div().text_xs().text_color(ui::muted())
                    .child("Only staged Git changes are committed; \
unstaged files stay untouched."))
                .child(Input::new(&self.git_summary).small())
                .child(
                    Button::new("remote-git-commit")
                        .primary().small().w_full()
                        .label(if confirmed { "Confirm commit" } else { "Review commit" })
                        .disabled(self.git_busy || self.git_loading
                            || summary.is_empty() || staged_count == 0
                            || snapshot.files_truncated)
                        .on_click(cx.listener(|this, _, _, cx| this.git_commit(cx))),
                )
                .when(snapshot.files_truncated, |v| v.child(
                    div().text_xs().text_color(ui::warning())
                        .child("Refresh in a smaller worktree before committing."),
                ))
                .into_any_element(),
        );
    }

    fn push_git_branch_row(
        &self, rows: &mut Vec<AnyElement>, branch: &RemoteGitBranch,
        index: usize, writable: bool, cx: &mut Context<Self>,
    ) {
        let name = branch.name.clone();
        let confirmed = self.git_confirm_switch.as_deref() == Some(name.as_str());
        rows.push(
            h_flex()
                .id(("remote-git-branch", index))
                .h(px(32.)).px_3().gap_2()
                .child(Icon::new(IconName::GitBranch).small()
                    .text_color(if branch.current {
                        ui::success()
                    } else { ui::muted() }))
                .child(div().flex_1().min_w_0().truncate().text_xs()
                    .font_weight(if branch.current {
                        FontWeight::SEMIBOLD
                    } else { FontWeight::NORMAL })
                    .child(branch.name.clone()))
                .when(branch.current, |row| row.child(
                    div().text_xs().text_color(ui::success())
                        .child("HEAD"),
                ))
                .when(writable && branch.local && !branch.current, |row| row.child(
                    Button::new(format!("remote-git-switch-{index}"))
                        .ghost().xsmall()
                        .label(if confirmed { "Confirm switch" } else { "Switch" })
                        .disabled(self.git_busy || self.git_loading)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.git_switch_branch(name.clone(), cx);
                        })),
                ))
                .into_any_element(),
        );
    }

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
    fn render_git_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(snapshot) = &self.git {
            let writable = snapshot.writes_enabled && self.root != "/";
            rows.push(
                div().px_3().py_2().text_xs().text_color(ui::muted())
                    .child(snapshot.toplevel.clone()).into_any_element(),
            );
            if self.tab == RemoteTab::Changes {
                if snapshot.files.is_empty() {
                    rows.push(div().px_3().py_3().text_sm().text_color(ui::muted())
                        .child("Working tree clean").into_any_element());
                }
                for side in [ChangeSide::Unstaged, ChangeSide::Staged] {
                    let entries = side_entries(&snapshot.files, side);
                    if entries.is_empty() {
                        continue;
                    }
                    self.push_git_section_header(&mut rows, side, entries.len());
                    for (index, badge) in entries {
                        if let Some(entry) = snapshot.files.get(index) {
                            self.push_git_file_row(
                                &mut rows, snapshot, index, entry, side, badge, writable, cx,
                            );
                        }
                    }
                }
                if snapshot.files_truncated {
                    rows.push(div().px_3().py_2().text_xs()
                        .text_color(ui::warning())
                        .child("Showing first 200 changed files").into_any_element());
                }
                if writable {
                    self.push_git_commit_form(&mut rows, snapshot, cx);
                }
            } else if self.tab == RemoteTab::Branches {
                if snapshot.branches.is_empty() {
                    rows.push(div().px_3().py_3().text_sm().text_color(ui::muted())
                        .child("No Git branches").into_any_element());
                }
                for (index, branch) in snapshot.branches.iter().enumerate() {
                    self.push_git_branch_row(&mut rows, branch, index, writable, cx);
                }
                if snapshot.branches_truncated {
                    rows.push(div().px_3().py_2().text_xs()
                        .text_color(ui::warning())
                        .child("Showing first 120 branches").into_any_element());
                }
                if writable {
                    self.push_git_create_branch_form(&mut rows, snapshot, cx);
                }
            }
        }
        v_flex().children(rows).into_any_element()
    }

    /// Keep the root GPUI render shallow on Windows debug builds.
    /// 5D previously built every interactive row and both editors inside
    /// this one render frame, overflowing the Windows UI thread stack.
    fn render_git_panel(&self, cx: &mut Context<Self>) -> Div {
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex().h(px(40.)).px_2().gap_1().flex_none()
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
            .child(
                div().id("remote-git-scroll").flex_1().min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_git_list(cx)),
            )
            .child(
                div().h(px(26.)).flex_none().border_t_1()
                    .border_color(ui::border()).px_3().text_size(px(10.))
                    .text_color(ui::muted())
                    .child(if self.git.as_ref().is_some_and(|s| s.writes_enabled) {
                        "Remote Git · guarded writes · no push/discard"
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
        let mut content = v_flex().gap(px(1.));
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
                                .child("128 KiB maximum")),
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
            for (index, entry) in self.entries.iter().enumerate() {
                let is_dir = entry.kind == "directory";
                let is_file = entry.kind == "file";
                let path = remote_child(&self.current, &entry.name);
                let click_path = path.clone();
                let insert_path = path;
                content = content.child(
                    h_flex()
                        .id(("remote-file-row", index))
                        .h(px(26.)).px_2().gap(px(5.))
                        .cursor_pointer()
                        .hover(|row| row.bg(ui::surface_hover()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if is_dir {
                                this.load_directory(click_path.clone(), cx);
                            } else if is_file {
                                cx.emit(RemoteFilesEvent::OpenFile(click_path.clone()));
                            }
                        }))
                        .child(
                            Icon::new(if is_dir { IconName::Folder }
                                      else { IconName::File })
                                .small().text_color(if is_dir { ui::champagne() } else { ui::muted() }),
                        )
                        .child(div().flex_1().min_w_0().truncate()
                            .text_xs().child(entry.name.clone()))
                        .child(
                            Button::new(("remote-file-insert", index))
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::Copy))
                                .tooltip("Insert Linux path into agent prompt")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(RemoteFilesEvent::InsertPath(insert_path.clone()));
                                })),
                        ),
                );
            }
        }

        v_flex()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::save_shortcut))
            .size_full()
            .min_h_0()
            .child(
                h_flex().h(px(40.)).px_2().gap_1().flex_none()
                    .child(Icon::new(if central { IconName::FileCode }
                        else { IconName::Layers })
                        .small().text_color(ui::petal()))
                    .child(div().flex_1().min_w_0().truncate()
                        .text_sm().font_weight(FontWeight::SEMIBOLD)
                        .child(if let Some(path) = &self.pane_path {
                            format!("{} · {}", self.machine, path)
                        } else {
                            format!("Remote Files · {}", self.machine)
                        }))
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
                div().px_3().pb_2().text_xs().text_color(ui::muted())
                    .child(self.current.clone()),
            ))
            .when(self.loading, |panel| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::muted())
                    .child("Reading from remote agent…"),
            ))
            .when_some(self.error.clone(), |panel, error| panel.child(
                div().px_3().py_2().text_xs().text_color(ui::warning())
                    .child(error),
            ))
            .child(
                div()
                    .id("remote-file-tree-scroll")
                    .flex_1().min_h_0().overflow_y_scroll()
                    .child(content),
            )
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
                        "Editing · SHA-256 conflict protection · 128 KiB"
                    } else {
                        "Files · preview first · 128 KiB maximum"
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_git_snapshot, remote_child, remote_parent};
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
