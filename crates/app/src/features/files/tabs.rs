//! Shared Files / Changes / Branches navigation for local and Herdr.
//! Backend choice must not affect tab geometry, colors or labels.

use gpui_kit::assets::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::shared::ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FilePanelTab {
    Files,
    Changes,
    Branches,
}

impl FilePanelTab {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Changes => "Changes",
            Self::Branches => "Branches",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Files => IconName::Files,
            Self::Changes => IconName::Diff,
            Self::Branches => IconName::GitBranch,
        }
    }
}

/// Render the same tab for both backends. The owning view attaches its
/// own click handler; no source-specific UI logic belongs here.
pub(super) fn tab_button(
    tab: FilePanelTab,
    active: FilePanelTab,
    count: usize,
) -> Stateful<Div> {
    let label = tab.label();
    h_flex()
        .id(label)
        .h(px(26.))
        .min_w(px(32.))
        .px_2()
        .gap_1()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .when(active == tab, |d| d.bg(ui::surface_hover()))
        .when(active != tab, |d| {
            d.hover(|s| s.bg(ui::surface_hover().opacity(0.6)))
        })
        .child(
            Icon::new(tab.icon())
                .small()
                .text_color(if active == tab { ui::fg() } else { ui::faint() }),
        )
        .when(count > 0, |d| {
            d.child(
                div()
                    .font_family(ui::MONO)
                    .text_size(px(10.5))
                    .text_color(ui::warning())
                    .child(count.to_string()),
            )
        })
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
}
