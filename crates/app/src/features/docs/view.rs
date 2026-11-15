//! Render, after Setup & requirements: groups under a section label, each an
//! island of rows (icon, title and line, action on the right). Asking the
//! agent is a list of rows; the two that need a name open in place.

use gentle_core::docs::Standing;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Ask, Doc, DocsEvent, DocsView, SkillState};
use crate::features::files::{DiagramRenderer, diagram_renderer, install_diagram_renderer};
use crate::shared::ui;

/// A section label over `body`.
fn labelled(label: &'static str, body: impl IntoElement) -> Div {
    v_flex()
        .gap_2()
        .child(div().px_1().child(ui::section_label(label)))
        .child(body)
}

/// A section label over an island of rows.
fn group(label: &'static str, rows: impl IntoElement, cx: &App) -> Div {
    labelled(label, ui::island(cx).p(px(6.)).child(rows))
}

/// One row: a 20 px icon slot, title and line, and what goes on the right.
fn row(icon: AnyElement, title: &'static str, line: impl Into<SharedString>) -> Div {
    h_flex()
        .gap_3()
        .px_3()
        .py_2p5()
        .rounded(px(ui::ISLAND_ROW))
        .child(
            div()
                .flex_none()
                .size_5()
                .flex()
                .items_center()
                .justify_center()
                .child(icon),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ui::fg())
                        .child(title),
                )
                .child(div().text_xs().text_color(ui::muted()).child(line.into())),
        )
}

fn tone_icon(name: IconName, color: Hsla) -> AnyElement {
    Icon::new(name).small().text_color(color).into_any_element()
}

fn standing_chip(standing: Standing) -> Div {
    let color = match standing {
        Standing::Current => ui::success(),
        Standing::Stale => ui::warning(),
        Standing::Unverified => ui::muted(),
        Standing::Issues => ui::danger(),
    };
    ui::chip(standing.label(), color)
}

impl Ask {
    fn icon(self) -> IconName {
        match self {
            Ask::Project => IconName::BookOpen,
            Ask::Feature => IconName::Layers,
            Ask::Decision => IconName::NotebookPen,
            Ask::Update => IconName::RefreshCw,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Ask::Project => "Document this project",
            Ask::Feature => "Document a feature",
            Ask::Decision => "Record a decision",
            Ask::Update => "Update after changes",
        }
    }

    fn line(self) -> &'static str {
        match self {
            Ask::Project => {
                "Maps the code and writes the structure, the index and every document it can ground in it. Shows the plan first."
            }
            Ask::Feature => "Its components, data model, states and flows.",
            Ask::Decision => {
                "An ADR from what was decided: why, the alternatives and what it costs."
            }
            Ask::Update => "Brings the documents whose code changed up to date.",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Ask::Project => "docs-ask-project",
            Ask::Feature => "docs-ask-feature",
            Ask::Decision => "docs-ask-decision",
            Ask::Update => "docs-ask-update",
        }
    }
}

impl DocsView {
    fn render_setup(&self, cx: &mut Context<Self>) -> Div {
        // The docs skill, in Gentle Shell.
        let skill_ok = self.skill_ready();
        let skill_line = match &self.skill {
            SkillState::Missing => {
                "Teaches the agent to write and keep these docs. Installs into Gentle Shell."
                    .to_string()
            }
            SkillState::Installing => "Installing into Gentle Shell…".to_string(),
            SkillState::JustInstalled => {
                "Installed. Agent tabs started from now on have it.".to_string()
            }
            SkillState::Ready(_) => "Agents can write, update and check the docs.".to_string(),
            SkillState::Failed(error) => format!("Couldn't install it: {error}"),
        };
        let skill_icon = match &self.skill {
            SkillState::Failed(_) => tone_icon(IconName::CircleX, ui::danger()),
            _ if skill_ok => tone_icon(IconName::CircleCheck, ui::success()),
            _ => tone_icon(IconName::BookOpen, ui::muted()),
        };
        let installing_skill = matches!(self.skill, SkillState::Installing);
        let skill = row(skill_icon, "Docs skill", skill_line)
            .when(skill_ok, |d| d.bg(ui::success().opacity(0.08)))
            .when(!skill_ok, |d| {
                d.child(
                    Button::new("docs-install-skill")
                        .small()
                        .primary()
                        .icon(Icon::new(if matches!(self.skill, SkillState::Failed(_)) {
                            IconName::RefreshCw
                        } else {
                            IconName::ArrowDown
                        }))
                        .label(if installing_skill {
                            "Installing…"
                        } else if matches!(self.skill, SkillState::Failed(_)) {
                            "Retry"
                        } else {
                            "Install"
                        })
                        .loading(installing_skill)
                        .disabled(installing_skill)
                        .on_click(cx.listener(|this, _, _, cx| this.install_skill(cx))),
                )
            });
        // The diagram renderer, in the app.
        let renderer = diagram_renderer(cx).clone();
        let renderer_ok = matches!(renderer, DiagramRenderer::Ready(_));
        let installing = matches!(renderer, DiagramRenderer::Installing(_));
        let renderer_line = match &renderer {
            DiagramRenderer::Missing => "Draws the docs' diagrams in the preview. Optional: a one-time download of about 20 MB, not part of the app.".to_string(),
            DiagramRenderer::Installing(progress) => progress.clone(),
            DiagramRenderer::Ready(_) => "Mermaid diagrams are drawn in the Markdown preview.".to_string(),
            DiagramRenderer::Failed(error) => format!("Couldn't install it: {error}"),
        };
        let renderer_icon = match &renderer {
            DiagramRenderer::Ready(_) => tone_icon(IconName::CircleCheck, ui::success()),
            DiagramRenderer::Failed(_) => tone_icon(IconName::CircleX, ui::danger()),
            _ => tone_icon(IconName::Workflow, ui::muted()),
        };
        let diagrams = row(renderer_icon, "Diagram renderer", renderer_line)
            .when(renderer_ok, |d| d.bg(ui::success().opacity(0.08)))
            .when(!renderer_ok, |d| {
                d.child(
                    Button::new("docs-install-renderer")
                        .small()
                        .ghost()
                        .icon(Icon::new(
                            if matches!(renderer, DiagramRenderer::Failed(_)) {
                                IconName::RefreshCw
                            } else {
                                IconName::ArrowDown
                            },
                        ))
                        .label(if installing {
                            "Installing…"
                        } else if matches!(renderer, DiagramRenderer::Failed(_)) {
                            "Retry"
                        } else {
                            "Install"
                        })
                        .loading(installing)
                        .disabled(installing)
                        .on_click(|_, _, cx| install_diagram_renderer(cx)),
                )
            });
        group("Set up", v_flex().gap_1().child(skill).child(diagrams), cx)
    }

    fn render_ask(&self, ask: Ask, cx: &mut Context<Self>) -> AnyElement {
        let ready = self.skill_ready();
        let open = self.asking == Some(ask);
        let icon = tone_icon(ask.icon(), if ready { ui::accent() } else { ui::faint() });
        let line = row(icon, ask.title(), ask.line()).when(!open, |d| {
            d.child(
                Icon::new(IconName::ChevronRight)
                    .xsmall()
                    .text_color(ui::faint()),
            )
        });
        if !open {
            return line
                .id(ask.id())
                .when(ready, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(ui::surface_hover()))
                        .on_click(cx.listener(move |this, _, window, cx| this.ask(ask, window, cx)))
                })
                .when(!ready, |d| d.opacity(0.6))
                .into_any_element();
        }
        let typed = !self.name.read(cx).value().trim().is_empty();
        v_flex()
            .rounded(px(ui::ISLAND_ROW))
            .bg(ui::surface_hover())
            .pb_2p5()
            .child(line)
            .child(
                h_flex()
                    .pl(px(44.))
                    .pr_3()
                    .gap_2()
                    .child(
                        ui::field()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.name).appearance(false).small()),
                    )
                    .child(
                        Button::new("docs-ask-cancel")
                            .small()
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_ask(cx))),
                    )
                    .child(
                        Button::new("docs-ask-run")
                            .small()
                            .outline()
                            .icon(Icon::new(IconName::ArrowRight))
                            .label("Ask")
                            .disabled(!typed)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_asks(&self, cx: &mut Context<Self>) -> Div {
        let rows: Vec<AnyElement> = Ask::ALL
            .into_iter()
            .map(|ask| self.render_ask(ask, cx))
            .collect();
        group("Ask the agent", v_flex().gap_1().children(rows), cx)
            .when(!self.skill_ready(), |d| {
                d.child(
                    div()
                        .px_1()
                        .text_xs()
                        .text_color(ui::faint())
                        .child("Install the docs skill to ask for these."),
                )
            })
            .when(self.skill_ready(), |d| {
                d.child(
                    div()
                        .px_1()
                        .text_xs()
                        .text_color(ui::faint())
                        .child("Each opens an agent tab in this folder, so you can follow along and answer its questions."),
                )
            })
    }

    fn render_docs(&self, cx: &mut Context<Self>) -> Div {
        let Some(docs) = &self.docs else {
            return labelled(
                "Documents",
                h_flex()
                    .px_1()
                    .gap_2()
                    .text_sm()
                    .text_color(ui::muted())
                    .child(Spinner::new().small().color(ui::accent()))
                    .child("Reading docs/…"),
            );
        };
        if docs.is_empty() {
            return labelled(
                "Documents",
                v_flex()
                    .items_center()
                    .py_8()
                    .gap_2()
                    .child(Icon::new(IconName::BookOpen).text_color(ui::faint()))
                    .child(
                        div().text_sm().text_color(ui::muted()).child(
                            "No docs yet. \u{201c}Document this project\u{201d} starts them.",
                        ),
                    ),
            );
        }
        let attention = docs
            .iter()
            .filter(|d| d.standing.is_some_and(|s| s != Standing::Current))
            .count();
        let summary = match (&self.check_note, attention) {
            (Some(note), _) => note.clone(),
            (None, 0) => format!(
                "{} documents, all current{}.",
                docs.len(),
                self.head
                    .as_ref()
                    .map(|h| format!(" at {h}"))
                    .unwrap_or_default()
            ),
            (None, n) => format!(
                "{} documents, {n} need attention: \u{201c}Update after changes\u{201d} catches them up.",
                docs.len()
            ),
        };
        let rows: Vec<AnyElement> = docs
            .iter()
            .enumerate()
            .map(|(ix, doc)| self.render_doc(ix, doc, cx).into_any_element())
            .collect();
        group("Documents", v_flex().gap_1().children(rows), cx).child(
            div()
                .px_1()
                .text_xs()
                .text_color(ui::faint())
                .child(summary),
        )
    }

    fn render_doc(&self, ix: usize, doc: &Doc, cx: &mut Context<Self>) -> Stateful<Div> {
        let path = doc.path.clone();
        let icon = match doc.standing {
            Some(Standing::Current) => tone_icon(IconName::FileText, ui::success()),
            Some(Standing::Stale) => tone_icon(IconName::FileText, ui::warning()),
            Some(Standing::Issues) => tone_icon(IconName::FileText, ui::danger()),
            _ => tone_icon(IconName::FileText, ui::muted()),
        };
        let line = match doc.notes.first() {
            Some(note) => format!("{} · {note}", doc.relative),
            None => doc.relative.clone(),
        };
        h_flex()
            .id(("docs-doc", ix))
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(ui::ISLAND_ROW))
            .cursor_pointer()
            .hover(|s| s.bg(ui::surface_hover()))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(DocsEvent::Open(path.clone()))))
            .child(
                div()
                    .flex_none()
                    .size_5()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(ui::fg())
                            .child(doc.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .font_family(ui::MONO)
                            .text_color(ui::faint())
                            .child(line),
                    ),
            )
            .children(doc.standing.map(standing_chip))
    }
}

impl Render for DocsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let folder = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string());
        let loading = self.docs.is_none();
        let (backdrop, panel) = ui::screen(Some(760.));
        backdrop.occlude().child(ui::pop_in(
            "docs-screen",
            panel
                .w_full()
                .max_w(px(760.))
                .child(
                    ui::screen_header()
                        .child(
                            Icon::new(IconName::BookOpen)
                                .small()
                                .text_color(ui::muted()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ui::fg())
                                .child("Architecture docs"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(ui::muted())
                                .child(format!(
                                    "{folder}/docs, written and kept current by the agent"
                                )),
                        )
                        .child(
                            Button::new("docs-refresh")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::RefreshCw))
                                .tooltip("Check the documents again")
                                .loading(loading)
                                .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                        )
                        .child(
                            Button::new("docs-close")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::X))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(DocsEvent::Close))),
                        ),
                )
                .child(
                    div()
                        .id("docs-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(
                            v_flex()
                                .gap_5()
                                .px_6()
                                .py_5()
                                .child(self.render_setup(cx))
                                .child(self.render_asks(cx))
                                .child(self.render_docs(cx)),
                        ),
                ),
        ))
    }
}
