//! Linux Files bound to one Herdr Pi agent. Remote preview and opt-in
//! revision-checked UTF-8 editing run through the existing injected bridge.
//! This view never uses local Windows filesystem APIs or remote Pi bash.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use crate::features::session::SessionView;
use crate::shared::ui;

pub enum RemoteFilesEvent {
    InsertPath(String),
    /// Open an existing remote Linux file in a normal central editor pane.
    OpenFile(String),
    Close,
}

#[derive(Clone)]
struct RemoteEntry {
    name: String,
    kind: String,
}

struct RemotePreview {
    path: String,
    content: String,
    /// Missing on old bridges: keep browsing read-only instead of inventing
    /// a revision that could overwrite a changed remote file.
    revision: Option<String>,
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
    focus: FocusHandle,
    entries: Vec<RemoteEntry>,
    preview: Option<RemotePreview>,
    editor: Option<Entity<EditorState>>,
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
        let mut view = Self {
            session,
            machine,
            current: root.clone(),
            root,
            pane_path: None,
            focus: cx.focus_handle(),
            entries: Vec::new(),
            preview: None,
            editor: None,
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

    pub fn file_title(&self) -> SharedString {
        let Some(path) = self.pane_path.as_ref() else {
            return "Remote Files".into();
        };
        let filename = path.rsplit('/').next().unwrap_or(path);
        format!("{filename} · {}", self.machine).into()
    }

    pub fn has_unsaved(&self, cx: &App) -> bool {
        self.saving || self.editor.as_ref().zip(self.preview.as_ref()).is_some_and(
            |(editor, preview)| editor.read(cx).text().to_string() != preview.content
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
            .is_some_and(|(editor, preview)| editor.read(cx).text().to_string() != preview.content)
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
        let response = process.request(json!({
            "type": "herdr_fs_list", "root": self.root,
            "path": directory
        }));
        cx.spawn(async move |this, cx| {
            let response = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match response {
                    Ok(reply) => {
                        let data = reply.data;
                        this.entries = data["entries"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|entry| {
                                Some(RemoteEntry {
                                    name: entry["name"].as_str()?.to_owned(),
                                    kind: entry["kind"].as_str()?.to_owned(),
                                })
                            })
                            .collect();
                        this.truncated = data["truncated"].as_bool().unwrap_or(false);
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
        self._editor_events = None;
        self.error = None;
        self.loading = true;
        let Some(process) = self.session.read(cx).remote_fs_process() else {
            self.loading = false;
            self.error = Some("Remote agent is not connected".into());
            cx.notify();
            return;
        };
        let response = process.request(json!({
            "type": "herdr_fs_read", "root": self.root, "path": path
        }));
        cx.spawn_in(window, async move |this, cx| {
            let response = response.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match response {
                    Ok(reply) => {
                        if let Some(content) = reply.data["content"].as_str() {
                            this.preview = Some(RemotePreview {
                                path,
                                content: content.to_owned(),
                                revision: reply.data["revision"].as_str()
                                    .filter(|r| r.len() == 64
                                        && r.bytes().all(|byte| byte.is_ascii_hexdigit()))
                                    .map(str::to_owned),
                            });
                            if this.pane_path.is_some() {
                                this.start_edit(window, cx);
                            }
                        } else {
                            this.error = Some("Invalid remote file response".into());
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
            |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event { cx.notify(); }
            }));
        self.editor = Some(editor);
        self.error = None;
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
            self.cancel_edit(cx);
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
        let command = json!({
            "type": "herdr_fs_write", "root": self.root,
            "path": path, "content": text, "expectedRevision": revision,
        });
        // A 128 KiB file takes many 96-byte PTY frames. The bridge request
        // writes synchronously, so start it off the GPUI foreground thread.
        let response = cx.background_executor().spawn(async move {
            process.request(command).await
        });
        cx.spawn(async move |this, cx| {
            let result = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation { return; }
                this.saving = false;
                match result {
                    Ok(reply) => {
                        if let Some(new_revision) = reply.data["revision"].as_str() {
                            this.preview = Some(RemotePreview {
                                path, content: text, revision: Some(new_revision.into()),
                            });
                            this.editor = None;
                            this._editor_events = None;
                            this.error = None;
                        } else {
                            this.error = Some("Save response had no revision; reload before editing again.".into());
                        }
                    }
                    Err(error) => this.error = Some(friendly_error(&error.to_string())),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_directory(self.current.clone(), cx);
    }
}

impl Render for RemoteFilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let parent = remote_parent(&self.root, &self.current);
        let central = self.pane_path.is_some();
        let mut content = v_flex().gap(px(1.));
        if let Some(preview) = &self.preview {
            let copied = preview.path.clone();
            let rows: Vec<String> = preview.content.lines().take(600).map(str::to_owned).collect();
            let truncated = preview.content.lines().count() > rows.len();
             let can_edit = preview.revision.is_some() && self.root != "/";
            let dirty = self.editor.as_ref().is_some_and(|editor| {
                editor.read(cx).text().to_string() != preview.content
            });
            content = content.child(
                h_flex()
                    .px_2()
                    .py_2()
                    .gap_1()
                    .child(
                        Button::new("remote-files-back-preview")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::ChevronLeft))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_directory(this.current.clone(), cx)
                            })),
                    )
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
                        .child(
                            Button::new("remote-files-cancel-edit")
                                .ghost().small().label("Cancel")
                                .disabled(self.saving)
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_edit(cx))),
                        )
                        .child(div().flex_1())
                        .child(div().text_xs().text_color(ui::muted())
                            .child(if dirty { "Unsaved changes" } else { "Unchanged" }))
                ).child(
                    div().px_2().py_2()
                        .h(px(if central {
                            (f32::from(window.viewport_size().height) - 162.).max(260.)
                        } else { 360. }))
                        .w_full().overflow_hidden()
                        .rounded_md().border_1().border_color(ui::border())
                        .child(
                            Editor::new(editor)
                                .appearance(false)
                                .disabled(self.saving)
                                .font_family(ui::MONO)
                                .text_size(px(12.5))
                                .size_full()
                        )
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
                    .child(Icon::new(IconName::Layers).small().text_color(ui::petal()))
                    .child(div().flex_1().min_w_0().truncate()
                        .text_sm().font_weight(FontWeight::SEMIBOLD)
                        .child(if let Some(path) = &self.pane_path {
                            format!("{} · {}", self.machine, path)
                        } else {
                            format!("Remote Files · {}", self.machine)
                        }))
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
                    ))
                    ,
            )
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
                        "Preview first · Edit to modify · 128 KiB maximum"
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{remote_child, remote_parent};
    #[test]
    fn remote_paths_remain_under_root() {
        assert_eq!(remote_child("/srv/workspaces/demo", "Cargo.toml"),
            "/srv/workspaces/demo/Cargo.toml");
        assert_eq!(remote_child("/", "srv"), "/srv");
        assert_eq!(remote_parent("/srv/workspaces/demo", "/srv/workspaces/demo/src"),
            Some("/srv/workspaces/demo".to_owned()));
        assert_eq!(remote_parent("/srv/workspaces/demo", "/srv/workspaces/demo"), None);
    }
}
