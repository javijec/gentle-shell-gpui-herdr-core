//! Shared visual primitives for a local or Herdr Files dock.
//! Backend-bound callbacks live in the owning view; no filesystem or Git IO.
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(super) fn panel_shell() -> Div {
    v_flex().size_full().min_h_0()
}

pub(super) fn panel_header() -> Div {
    h_flex().h(px(40.)).px_2().gap_1().flex_none()
}

pub(super) fn file_row_shell(
    id: SharedString, depth: usize, highlighted: bool,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(26.))
        .w_full()
        .pl(px(6. + depth as f32 * 14.))
        .pr_2()
        .gap_1p5()
        .rounded_md()
        .when(highlighted, |d| d.bg(crate::shared::ui::raised()))
        .when(!highlighted, |d| d.hover(|s| s.bg(crate::shared::ui::surface_hover())))
}

/// Fully shared file-entry row (native and remote). Backend controllers
/// attach their own click, insert-path, rename and context-menu handlers.
#[allow(clippy::too_many_arguments)]
pub(super) fn file_entry_row(
    id: SharedString, depth: usize, selected: bool,
    name: &str, is_dir: bool, opened: bool, dimmed: bool,
    status: Option<char>, status_label: Option<char>, dirty: bool,
) -> Stateful<Div> {
    let (icon, tint) = if is_dir {
        (if opened {
            gpui_kit::assets::IconName::FolderOpen
        } else {
            gpui_kit::assets::IconName::Folder
        }, crate::shared::ui::muted())
    } else {
        super::file_icon(name)
    };
    let name_color = match status {
        Some(letter) if !dimmed => super::status_color(letter),
        _ if dimmed => crate::shared::ui::faint(),
        _ => crate::shared::ui::fg(),
    };
    file_row_shell(id, depth, selected)
        .child(div().w(px(14.)).flex().justify_center()
            .when(is_dir, |d| d.child(
                gpui_kit::component::Icon::new(if opened {
                    gpui_kit::assets::IconName::ChevronDown
                } else {
                    gpui_kit::assets::IconName::ChevronRight
                })
                .xsmall().text_color(crate::shared::ui::faint()),
            )))
        .child(gpui_kit::component::Icon::new(icon)
            .small()
            .text_color(if dimmed { tint.opacity(0.55) } else { tint }))
        .child(div()
            .flex_1().min_w_0().truncate()
            .text_size(px(13.))
            .text_color(name_color)
            .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
            .child(name.to_owned()))
        .when(dirty, |d| d.child(
            div().size(px(5.)).rounded_full()
                .bg(crate::shared::ui::warning().opacity(0.8)),
        ))
        .when_some(status_label, |d, letter| d.child(
            div().font_family(crate::shared::ui::MONO)
                .text_size(px(10.5))
                .text_color(super::status_color(status.unwrap_or(letter)))
                .child(letter.to_string()),
        ))
}
