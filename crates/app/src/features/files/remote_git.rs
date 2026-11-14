//! Read-only Git diff pane, always bound to one existing Herdr agent.
//! No local Git invocation, Windows paths or remote mutations.

use gpui_kit::assets::IconName;
use gpui_kit::component::diff::{Diff, DiffFile, DiffHoverHighlight, DiffState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use serde_json::json;

use crate::features::session::SessionView;
use crate::shared::ui;

pub struct RemoteGitDiffView {
    machine: String,
    path: String,
    focus: FocusHandle,
    diff: Option<Entity<DiffState>>,
    error: Option<String>,
    loading: bool,
    _query: Task<()>,
}

impl RemoteGitDiffView {
    pub fn new(
        session: Entity<SessionView>,
        machine: String,
        root: String,
        path: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            machine,
            path: path.clone(),
            focus: cx.focus_handle(),
            diff: None,
            error: None,
            loading: false,
            _query: Task::ready(()),
        };
        let Some(process) = session.read(cx).remote_fs_process() else {
            view.error = Some("Remote agent is not connected. Reopen the diff after reconnection.".into());
            return view;
        };
        view.loading = true;
        let response = process.request(json!({
            "type": "herdr_git_diff", "root": root, "path": path,
        }));
        view._query = cx.spawn(async move |this, cx| {
            let response = response.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match response {
                    Ok(reply) => {
                        let Some(patch) = reply.data["patch"].as_str() else {
                            this.error = Some("Invalid remote diff response".into());
                            cx.notify();
                            return;
                        };
                        match DiffFile::parse(patch) {
                            Ok(files) => {
                                if let Some(file) = files.into_iter().next() {
                                    if file.is_binary() {
                                        this.error = Some("Binary changes cannot be shown as text.".into());
                                    } else {
                                        this.diff = Some(cx.new(|cx| DiffState::new([file], cx)));
                                    }
                                } else {
                                    this.error = Some("No textual differences for this file.".into());
                                }
                            }
                            Err(error) => this.error = Some(format!("Invalid Git patch: {error}")),
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        });
        view
    }

    pub fn title(&self) -> SharedString {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        format!("{name} · diff · {}", self.machine).into()
    }
}

impl Focusable for RemoteGitDiffView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.diff.as_ref().map_or_else(
            || self.focus.clone(),
            |diff| diff.focus_handle(cx),
        )
    }
}

impl Render for RemoteGitDiffView {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = if let Some(diff) = &self.diff {
            Diff::new(diff)
                .header_visible(false)
                .hover_highlight(DiffHoverHighlight::Line)
                .size_full()
                .into_any_element()
        } else if self.loading {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(Spinner::new().small().color(ui::muted()))
                .child(div().text_sm().text_color(ui::muted())
                    .child("Reading Git diff from JaviPi…"))
                .into_any_element()
        } else {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .px_4()
                .child(div().text_sm().text_color(ui::muted())
                    .child(self.error.clone().unwrap_or_else(|| "No diff available".into())))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(38.))
                    .px_4()
                    .flex_none()
                    .gap_2()
                    .child(Icon::new(IconName::Diff).small().text_color(ui::info()))
                    .child(div().flex_1().min_w_0().truncate()
                        .text_sm().font_weight(FontWeight::SEMIBOLD)
                        .child(self.path.clone()))
                    .child(ui::chip("Remote · read-only", ui::muted())),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(body))
    }
}
