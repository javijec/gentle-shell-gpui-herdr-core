//! Read-only Linux file browser bound to one Herdr Pi agent.
//! Unlike FileTreeView, no Windows filesystem or Git APIs are used here.
//! Only herdr_fs_list / herdr_fs_read are sent to that agent's Node bridge.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::json;

use crate::features::session::SessionView;
use crate::shared::ui;

pub enum RemoteFilesEvent {
    InsertPath(String),
    Close,
}

#[derive(Clone)]
struct RemoteEntry {
    name: String,
    kind: String,
}

pub struct RemoteFilesView {
    session: Entity<SessionView>,
    machine: String,
    root: String,
    current: String,
    entries: Vec<RemoteEntry>,
    /// A preview is text-only; there is deliberately no edit/save action.
    preview: Option<(String, String)>,
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
        "This running agent has an older Herdr bridge. Open a new remote agent to enable read-only files; the existing agent is preserved.".into()
    } else {
        error.into()
    }
}

impl RemoteFilesView {
    pub fn new(
        session: Entity<SessionView>,
        machine: String,
        root: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            session,
            machine,
            current: root.clone(),
            root,
            entries: Vec::new(),
            preview: None,
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
            view._wait_for_agent = cx.spawn(async move |this, cx| {
                for _ in 0..45 {
                    cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                    let ready = this.update(cx, |this, cx| {
                        if this.session.read(cx).remote_fs_process().is_some() {
                            this.load_directory(this.root.clone(), cx);
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
        &self.session == session && self.root == root
    }

    pub fn session(&self) -> &Entity<SessionView> {
        &self.session
    }

    fn load_directory(&mut self, directory: String, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.current = directory.clone();
        self.preview = None;
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

    fn open(&mut self, path: String, is_directory: bool, cx: &mut Context<Self>) {
        if is_directory {
            self.load_directory(path, cx);
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.preview = None;
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
        cx.spawn(async move |this, cx| {
            let response = response.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match response {
                    Ok(reply) => {
                        if let Some(content) = reply.data["content"].as_str() {
                            this.preview = Some((path, content.to_owned()));
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

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_directory(self.current.clone(), cx);
    }
}

impl Render for RemoteFilesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let parent = remote_parent(&self.root, &self.current);
        let mut content = v_flex().gap(px(1.));
        if let Some((path, text)) = &self.preview {
            let copied = path.clone();
            let rows: Vec<String> = text.lines().take(600).map(str::to_owned).collect();
            let truncated = text.lines().count() > rows.len();
            content = content
                .child(
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
                            .text_xs().child(path.clone()))
                        .child(
                            Button::new("remote-files-insert-preview")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::Copy))
                                .tooltip("Insert remote Linux path in prompt")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(RemoteFilesEvent::InsertPath(copied.clone()));
                                })),
                        ),
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
                            if is_dir || is_file {
                                this.open(click_path.clone(), is_dir, cx);
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
            .size_full()
            .min_h_0()
            .child(
                h_flex().h(px(40.)).px_2().gap_1().flex_none()
                    .child(Icon::new(IconName::Layers).small().text_color(ui::petal()))
                    .child(div().flex_1().min_w_0().truncate()
                        .text_sm().font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Remote Files · {}", self.machine)))
                    .child(
                        Button::new("remote-files-refresh")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::RefreshCw))
                            .tooltip("Refresh this remote directory")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .child(
                        Button::new("remote-files-close")
                            .ghost().xsmall()
                            .icon(Icon::new(IconName::X))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RemoteFilesEvent::Close))),
                    ),
            )
            .child(
                div().px_3().pb_2().text_xs().text_color(ui::muted())
                    .child(self.current.clone()),
            )
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
                    .child("Read-only · Files only · 128 KiB preview limit"),
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
