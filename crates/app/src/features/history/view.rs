//! The Commit History screen: header with the branch picker, the graph list,
//! the hover card of a commit, and the footer.

use std::rc::Rc;

use gentle_core::git::log::{Commit, RefLabel, initials};
use gentle_core::sessions::relative_time;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::graph::{self, RowGraph};
use super::{HistoryView, Line};
use crate::shared::ui;

const ROW_HEIGHT: f32 = 30.;
const AUTHOR_WIDTH: f32 = 150.;
const TIME_WIDTH: f32 = 40.;
/// Below these pane widths the row drops the author's name, then the author
/// and the ref chips.
const WIDE: f32 = 640.;
const MEDIUM: f32 = 460.;

/// Which columns fit the pane.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    Narrow,
    Medium,
    Wide,
}

/// At most `max` characters, with an ellipsis when cut.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

/// `2d ago` → `2d`, `just now` → `now`: the row's time column.
fn short_time(now_ms: i64, secs: i64) -> String {
    let text = relative_time(now_ms, secs * 1000);
    match text.strip_suffix(" ago") {
        Some(short) => short.to_string(),
        None => "now".to_string(),
    }
}

/// Initials in a circle; a petal dot marks a commit written with a co-author.
fn avatar(name: &str, coauthored: bool, size: f32) -> Div {
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(ui::surface())
        .border_1()
        .border_color(ui::border())
        .flex()
        .items_center()
        .justify_center()
        .text_size(px((size * 0.4).round()))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ui::muted())
        .child(initials(name))
        .when(coauthored, |d| {
            d.child(
                div()
                    .absolute()
                    .right(px(-2.))
                    .bottom(px(-2.))
                    .size(px((size * 0.4).max(7.)))
                    .rounded_full()
                    .bg(ui::petal())
                    .border_2()
                    .border_color(ui::screen_bg()),
            )
        })
}

impl HistoryView {
    /// The branch the screen shows, for the header and the hover card.
    fn shown_branch(&self) -> Option<String> {
        self.history
            .as_ref()
            .and_then(|h| h.as_ref())
            .and_then(|h| h.branch.clone())
    }

    fn fit(&self) -> Fit {
        match self.width.get() {
            // Not measured yet: the first frame lays out wide.
            w if w == 0. || w >= WIDE => Fit::Wide,
            w if w >= MEDIUM => Fit::Medium,
            _ => Fit::Narrow,
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> Div {
        let fit = self.fit();
        let history = self.history.as_ref().and_then(|h| h.as_ref());
        let total = history.map(|h| h.total);
        let branches = history.map(|h| h.branches.clone()).unwrap_or_default();
        let shown = self.shown_branch();
        let label = shown.clone().unwrap_or_else(|| "Detached HEAD".into());
        let folder = self
            .toplevel
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let weak = cx.entity().downgrade();
        // The pane header, as the file viewer's: no line under it.
        h_flex()
            .flex_none()
            .h(px(38.))
            .px_4()
            .gap_2()
            .child(
                Icon::new(IconName::GitGraph)
                    .small()
                    .text_color(ui::muted()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui::fg())
                    .child("Commit History"),
            )
            .when(fit > Fit::Narrow, |d| {
                d.when_some(total, |d, total| {
                    d.child(ui::chip(total.to_string(), ui::muted()).flex_shrink_0())
                })
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .text_color(ui::faint())
                    .when(fit > Fit::Narrow, |d| d.child(folder)),
            )
            .when(self.loading && history.is_some(), |d| {
                d.child(Spinner::new().xsmall().color(ui::muted()))
            })
            .when(!branches.is_empty(), |d| {
                d.child(
                    Button::new("history-branch")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::GitBranch))
                        // A narrow pane keeps the picker, not the long name.
                        .when(fit == Fit::Narrow, |b| b.tooltip(label.clone()))
                        .when(fit > Fit::Narrow, |b| {
                            b.label(clip(&label, if fit == Fit::Wide { 32 } else { 18 }))
                        })
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for branch in &branches {
                                let weak = weak.clone();
                                let name = branch.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(SharedString::from(branch.clone()))
                                        .checked(shown.as_ref() == Some(branch))
                                        .on_click(move |_, _, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                this.show_branch(name.clone(), cx)
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                )
            })
            .child(
                Button::new("history-refresh")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::RefreshCw))
                    .loading(self.loading)
                    .tooltip("Refresh")
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            )
    }

    fn graph_cell(&self, graph: RowGraph) -> Div {
        div()
            .relative()
            .flex_none()
            .w(px(graph::width(self.graph.width.max(1))))
            .h_full()
            .child(graph.element())
    }

    fn render_line(&self, ix: usize, line: Line, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .px_2()
            .child(self.render_row(ix, line, cx))
            .into_any_element()
    }

    fn render_row(&self, ix: usize, line: Line, cx: &mut Context<Self>) -> AnyElement {
        let row = h_flex()
            .id(("history-row", ix))
            .w_full()
            .h(px(ROW_HEIGHT))
            .pr_3()
            .gap_2()
            .rounded_lg();
        match line {
            Line::Uncommitted => row
                .child(self.graph_cell(RowGraph::uncommitted()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(ui::faint())
                        .child("Uncommitted changes"),
                )
                .into_any_element(),
            Line::Earlier(count) => row
                .cursor_pointer()
                .hover(|s| s.bg(ui::surface_hover()))
                .on_click(cx.listener(|this, _, _, cx| this.load_earlier(cx)))
                .child(
                    self.graph_cell(RowGraph::earlier()).child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .w(px(graph::PAD * 2.))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .h(px(18.))
                                    .min_w(px(18.))
                                    .px_1()
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(ui::info().opacity(0.16))
                                    .font_family(ui::MONO)
                                    .text_size(px(10.))
                                    .text_color(ui::info())
                                    .child(count.to_string()),
                            ),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(ui::faint())
                        .child("Earlier history"),
                )
                .when(self.loading, |d| {
                    d.child(Spinner::new().xsmall().color(ui::muted()))
                })
                .into_any_element(),
            Line::Commit(i) => self.render_commit(row, i, cx),
        }
    }

    fn render_commit(&self, row: Stateful<Div>, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let commit = &self.commits[i];
        let Some(graph_row) = self.graph.rows.get(i) else {
            return row.into_any_element();
        };
        let shown = self.shown_branch();
        let dirty_above = i == 0 && matches!(self.lines.first(), Some(Line::Uncommitted));
        let selected = self.selected.as_ref() == Some(&commit.hash);
        let lane_color = graph::lane_color(graph_row.color);
        let fit = self.fit();

        // Ref chips need room beside the subject: a narrow pane drops them
        // (the hover card still names the branch), others cut long names.
        let chip_max = if fit == Fit::Wide { 32 } else { 20 };
        let mut chips: Vec<AnyElement> = Vec::new();
        if fit > Fit::Narrow {
            for label in &commit.refs {
                match label {
                    RefLabel::Tag(tag) => {
                        chips.push(ui::chip(clip(tag, chip_max), ui::info()).into_any_element())
                    }
                    RefLabel::Branch(name) if shown.as_ref() != Some(name) => {
                        chips.push(ui::chip(clip(name, chip_max), lane_color).into_any_element())
                    }
                    _ => {}
                }
            }
            if graph_row.lane_tip
                && let Some(name) = &graph_row.branch
                && !commit.refs.contains(&RefLabel::Branch(name.clone()))
            {
                chips.push(ui::chip(clip(name, chip_max), lane_color).into_any_element());
            }
        }

        let card = CommitCard {
            commits: self.commits.clone(),
            index: i,
            branch: graph_row.branch.clone().or_else(|| shown.clone()),
            now_ms: self.now_ms,
        };
        let hash = commit.hash.clone();
        row.cursor_pointer()
            .when(selected, |d| d.bg(ui::raised()))
            .when(!selected, |d| d.hover(|s| s.bg(ui::surface_hover())))
            .on_click(cx.listener(move |this, _, _, cx| this.select(hash.clone(), cx)))
            .tooltip(move |_, cx| cx.new(|_| card.clone()).into())
            .child(self.graph_cell(RowGraph::commit(graph_row, commit.head, dirty_above)))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    // Chips that still don't fit are cut here, never drawn
                    // over the author.
                    .overflow_hidden()
                    .gap_2()
                    .child(
                        div()
                            // Without chips beside it, the subject takes the row.
                            .when(chips.is_empty(), |d| d.flex_1())
                            .min_w(px(96.))
                            .truncate()
                            .text_sm()
                            .text_color(if graph_row.mainline {
                                ui::fg()
                            } else {
                                ui::muted()
                            })
                            .child(commit.subject.clone()),
                    )
                    .children(chips.into_iter().map(|c| div().flex_none().child(c))),
            )
            .when(fit > Fit::Narrow, |d| {
                d.child(
                    h_flex()
                        .flex_none()
                        .when(fit == Fit::Wide, |d| d.w(px(AUTHOR_WIDTH)))
                        .gap_2()
                        .child(avatar(&commit.author, !commit.coauthors.is_empty(), 18.))
                        .when(fit == Fit::Wide, |d| {
                            d.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(ui::muted())
                                    .child(commit.author.clone()),
                            )
                        }),
                )
            })
            .child(
                div()
                    .flex_none()
                    .w(px(TIME_WIDTH))
                    .whitespace_nowrap()
                    .text_right()
                    .font_family(ui::MONO)
                    .text_xs()
                    .text_color(ui::faint())
                    .child(short_time(self.now_ms, commit.time)),
            )
            .into_any_element()
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = |icon: IconName, text: &'static str| {
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .py_8()
                .gap_2()
                .child(Icon::new(icon).text_color(ui::faint()))
                .child(div().text_sm().text_color(ui::muted()).child(text))
                .into_any_element()
        };
        match &self.history {
            None => h_flex()
                .flex_1()
                .justify_center()
                .py_8()
                .gap_2()
                .text_sm()
                .text_color(ui::muted())
                .child(Spinner::new().small().color(ui::muted()))
                .child("Reading history…")
                .into_any_element(),
            Some(None) => state(
                IconName::FolderGit,
                "This folder isn't in a Git repository.",
            ),
            Some(Some(_)) if self.commits.is_empty() => state(
                IconName::GitCommitHorizontal,
                "No commits on this branch yet.",
            ),
            Some(Some(_)) => div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    uniform_list(
                        "history-list",
                        self.lines.len(),
                        cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                            let lines = this.lines.clone();
                            range
                                .map(|ix| this.render_line(ix, lines[ix], cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full()
                    .py_2()
                    .track_scroll(&self.scroll),
                )
                .vertical_scrollbar(&self.scroll)
                .into_any_element(),
        }
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> Div {
        let selected = self
            .selected
            .as_ref()
            .and_then(|hash| self.commits.iter().find(|c| &c.hash == hash));
        let status: AnyElement = match (selected, self.history.as_ref().and_then(|h| h.as_ref())) {
            (Some(commit), _) => h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .flex_none()
                        .font_family(ui::MONO)
                        .text_color(ui::fg())
                        .child(commit.short().to_string()),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(ui::muted())
                        .child(commit.subject.clone()),
                )
                .into_any_element(),
            (None, Some(history)) if !self.commits.is_empty() => div()
                .flex_1()
                .text_xs()
                .text_color(ui::muted())
                .child(format!(
                    "Showing {} of {} commits",
                    self.commits.len(),
                    history.total
                ))
                .into_any_element(),
            _ => div().flex_1().into_any_element(),
        };
        // Inside a pane: separated by space, no line (the chrome has none).
        h_flex()
            .flex_none()
            .h(px(38.))
            .px_4()
            .gap_2()
            .child(status)
            .child(
                Button::new("history-copy")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Copy))
                    .when(self.fit() == Fit::Narrow, |b| b.tooltip("Copy hash"))
                    .when(self.fit() > Fit::Narrow, |b| b.label("Copy hash"))
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.copy_selected(cx))),
            )
    }
}

impl Render for HistoryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.width.clone();
        let view = cx.entity_id();
        v_flex()
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .min_h_0()
            // Measures the pane, so the next frame lays the rows out for it.
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let w = f32::from(bounds.size.width);
                        if (w - width.get()).abs() > 1. {
                            width.set(w);
                            cx.notify(view);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(self.render_header(cx))
            .child(self.render_body(cx))
            .child(self.render_footer(cx))
    }
}

/// The card a commit row shows on hover: author and co-authors, the full
/// subject, then hash, changes, time and branch.
#[derive(Clone)]
struct CommitCard {
    commits: Rc<Vec<Commit>>,
    index: usize,
    branch: Option<String>,
    now_ms: i64,
}

impl Render for CommitCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let commit = &self.commits[self.index];
        let detail = |label: &'static str, value: AnyElement| {
            h_flex()
                .gap_4()
                .justify_between()
                .text_xs()
                .child(div().flex_none().text_color(ui::muted()).child(label))
                .child(value)
        };
        let text = |value: String| {
            div()
                .min_w_0()
                .truncate()
                .text_color(ui::fg())
                .child(value)
                .into_any_element()
        };
        let changed = match commit.stat {
            Some(stat) => h_flex()
                .gap_1p5()
                .child(div().text_color(ui::fg()).child(format!(
                    "{} {}",
                    stat.files,
                    if stat.files == 1 { "file" } else { "files" }
                )))
                .child(
                    div()
                        .font_family(ui::MONO)
                        .text_color(ui::success())
                        .child(format!("+{}", stat.insertions)),
                )
                .child(
                    div()
                        .font_family(ui::MONO)
                        .text_color(ui::danger())
                        .child(format!("\u{2212}{}", stat.deletions)),
                )
                .into_any_element(),
            None if commit.is_merge() => text(format!("Merge of {} parents", commit.parents.len())),
            None => text("Nothing".into()),
        };
        let coauthors = (!commit.coauthors.is_empty())
            .then(|| format!("Co-authored with {}", commit.coauthors.join(", ")));
        ui::island(cx).w(px(320.)).child(
            v_flex()
                .p_3()
                .gap_3()
                .child(
                    h_flex()
                        .gap_3()
                        .child(avatar(&commit.author, coauthors.is_some(), 30.))
                        .child(
                            v_flex()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(ui::fg())
                                        .child(commit.author.clone()),
                                )
                                .when_some(coauthors, |d, line| {
                                    d.child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(ui::muted())
                                            .child(line),
                                    )
                                }),
                        ),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(ui::fg())
                        .child(commit.subject.clone()),
                )
                .child(
                    v_flex()
                        .gap_1p5()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(ui::well())
                        .child(detail(
                            "Commit",
                            div()
                                .font_family(ui::MONO)
                                .text_color(ui::fg())
                                .child(commit.short().to_string())
                                .into_any_element(),
                        ))
                        .child(detail("Changed", changed))
                        .child(detail(
                            "Committed",
                            text(relative_time(self.now_ms, commit.time * 1000)),
                        ))
                        .when_some(self.branch.clone(), |d, branch| {
                            d.child(detail("Branch", text(branch)))
                        }),
                ),
        )
    }
}
