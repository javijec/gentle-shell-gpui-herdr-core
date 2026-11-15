//! Shared Changes presentation: local and Herdr render the same rows and
//! staged/unstaged sections. Actions remain delegated to WorkspaceBackend.

use gpui_kit::*;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;

use gentle_core::git::status::StatusEntry;
use super::status_color;
use crate::shared::ui;

pub(super) const ROW_HEIGHT: f32 = 26.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ChangeSide {
    Unstaged,
    Staged,
}

impl ChangeSide {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Unstaged => "Unstaged",
            Self::Staged => "Staged",
        }
    }

    pub(super) fn action_label(self) -> &'static str {
        match self {
            Self::Unstaged => "Stage all",
            Self::Staged => "Unstage all",
        }
    }
}

pub(super) fn side_entries(entries: &[StatusEntry], side: ChangeSide) -> Vec<(usize, char)> {
    entries.iter().enumerate().filter_map(|(i, entry)| {
        match side {
            ChangeSide::Unstaged => entry.unstaged(),
            ChangeSide::Staged => entry.staged(),
        }.map(|badge| (i, badge))
    }).collect()
}

/// Shared section title and count; each backend provides controls and actions.
pub(super) fn section_header(
    side: ChangeSide, count: usize,
    control: impl IntoElement, action: impl IntoElement,
) -> Div {
    h_flex()
        .h(px(ROW_HEIGHT)).px_3().gap_2()
        .child(control)
        .child(div()
            .text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD)
            .text_color(ui::fg()).child(side.label()))
        .child(div()
            .px_1p5().rounded_md().bg(ui::well())
            .font_family(ui::MONO).text_size(px(10.5))
            .text_color(ui::muted()).child(count.to_string()))
        .child(div().flex_1())
        .child(action)
}

/// Shared status row: the containing view supplies the checkbox,
/// navigation callback and context menu, without converting remote paths.
pub(super) fn change_row(
    id: SharedString, path: &str, badge: char, selected: bool,
    indent: f32, control: impl IntoElement,
) -> Stateful<Div> {
    let (dir, name) = match path.rsplit_once('/') {
        Some((dir, name)) if indent == 0. => (format!("{dir}/"), name.to_owned()),
        Some((_, name)) => (String::new(), name.to_owned()),
        None => (String::new(), path.to_owned()),
    };
    let deleted = badge == 'D';
    h_flex()
        .id(id)
        .h(px(ROW_HEIGHT))
        .mx_1p5()
        .pl(px(6. + indent))
        .pr_1p5()
        .gap_2()
        .rounded_md()
        .cursor_pointer()
        .when(selected, |d| d.bg(ui::raised()))
        .when(!selected, |d| d.hover(|s| s.bg(ui::surface_hover())))
        .child(control)
        .child(status_badge(badge))
        .child(h_flex()
            .flex_1().min_w_0().text_size(px(13.))
            .when(!dir.is_empty(), |d| {
                d.child(div().min_w_0().truncate()
                    .text_color(ui::faint()).child(dir))
            })
            .child(div()
                .flex_none().max_w_full().truncate()
                .text_color(if deleted { ui::muted() } else { ui::fg() })
                .when(deleted, |d| d.line_through())
                .child(name)))
}

fn status_badge(badge: char) -> Div {
    let color = if badge == 'U' {
        ui::danger()
    } else {
        status_color(badge)
    };
    div()
        .flex_none()
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(color.opacity(0.5))
        .bg(color.opacity(0.1))
        .font_family(ui::MONO)
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(badge.to_string())
}


#[cfg(test)]
mod tests {
    use super::{ChangeSide, side_entries};
    use gentle_core::git::status::StatusEntry;

    #[test]
    fn one_staged_unstaged_projection_for_native_and_herdr() {
        let entries = vec![
            StatusEntry::from_porcelain_v1("one.txt".into(), 'M', 'M'),
            StatusEntry::from_porcelain_v1("two.txt".into(), '?', '?'),
            StatusEntry::from_porcelain_v1("three.txt".into(), 'A', ' '),
        ];
        assert_eq!(side_entries(&entries, ChangeSide::Unstaged), vec![(0, 'M'), (1, '?')]);
        assert_eq!(side_entries(&entries, ChangeSide::Staged), vec![(0, 'M'), (2, 'A')]);
        assert_eq!(ChangeSide::Unstaged.label(), "Unstaged");
        assert_eq!(ChangeSide::Staged.action_label(), "Unstage all");
    }
}
