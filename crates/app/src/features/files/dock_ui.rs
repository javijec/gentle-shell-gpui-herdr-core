//! Shared visual primitives for a local or Herdr Files dock.
//! Backend-bound callbacks live in the owning view; no filesystem or Git IO.
use gpui_kit::component::{h_flex, v_flex};
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
