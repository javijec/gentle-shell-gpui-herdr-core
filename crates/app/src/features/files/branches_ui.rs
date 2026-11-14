//! Branch rows shared by native Git and Herdr.
 //!
 //! Actions belong to the owning view/backend; the UI never executes Git.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::features::git::RefEntry;
use crate::shared::ui;

pub(super) struct BranchDisplay<'a> {
    pub name: &'a str,
    pub current: bool,
    pub gone: bool,
    pub ahead: u32,
    pub behind: u32,
}

impl<'a> BranchDisplay<'a> {
    pub(super) fn from_local(branch: &'a RefEntry) -> Self {
        Self {
            name: &branch.name,
            current: branch.current,
            gone: branch.gone,
            ahead: branch.ahead,
            behind: branch.behind,
        }
    }

    /// 5D's Herdr bridge reports a name and HEAD marker but no upstream
    /// tracking counts. A missing metric is not presented as a claimed value.
    pub(super) fn from_remote(name: &'a str, current: bool) -> Self {
        Self { name, current, gone: false, ahead: 0, behind: 0 }
    }
}

pub(super) fn branch_row(
    id: SharedString, branch: BranchDisplay<'_>,
    action: Option<AnyElement>,
) -> Stateful<Div> {
    let trailing = if branch.gone {
        Some(ui::chip("gone", ui::warning()).into_any_element())
    } else if branch.ahead > 0 || branch.behind > 0 {
        Some(ahead_behind(branch.ahead, branch.behind).into_any_element())
    } else if branch.current {
        Some(
            Icon::new(IconName::Check)
                .xsmall()
                .text_color(ui::accent())
                .into_any_element(),
        )
    } else {
        None
    };
    h_flex()
        .id(id)
        .h(px(26.))
        .mx_1p5()
        .pl(px(24.))
        .pr_1p5()
        .gap_2()
        .rounded_md()
        .hover(|s| s.bg(ui::surface_hover()))
        .child(Icon::new(IconName::GitBranch).xsmall().text_color(
            if branch.current { ui::accent() } else { ui::muted() },
        ))
        .child(div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_size(px(13.))
            .text_color(if branch.current { ui::fg() } else { ui::fg().opacity(0.85) })
            .when(branch.current, |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(branch.name.to_owned()))
        .children(trailing)
        .children(action)
}

fn ahead_behind(ahead: u32, behind: u32) -> Div {
    h_flex()
        .flex_none()
        .gap_1p5()
        .font_family(ui::MONO)
        .text_size(px(11.))
        .child(div()
            .text_color(if ahead > 0 { ui::fg() } else { ui::muted() })
            .child(format!("↑{ahead}")))
        .child(div()
            .text_color(if behind > 0 { ui::fg() } else { ui::muted() })
            .child(format!("↓{behind}")))
}

#[cfg(test)]
mod tests {
    use super::BranchDisplay;

    #[test]
    fn herdr_branch_display_does_not_invent_tracking_metrics() {
        let branch = BranchDisplay::from_remote("feature/x", false);
        assert_eq!(branch.name, "feature/x");
        assert!(!branch.current);
        assert!(!branch.gone);
        assert_eq!((branch.ahead, branch.behind), (0, 0));
    }

    #[test]
    fn local_branch_display_retains_tracking_status() {
        use crate::features::git::RefEntry;
        use gentle_core::git::refs::RefKind;
        let ref_entry = RefEntry {
            kind: RefKind::Branch,
            name: "main".into(),
            current: true,
            upstream: Some("origin/main".into()),
            ahead: 2,
            behind: 1,
            gone: false,
            sha: "abcdef0".into(),
            subject: "Commit".into(),
        };
        let branch = BranchDisplay::from_local(&ref_entry);
        assert_eq!((branch.ahead, branch.behind), (2, 1));
        assert!(branch.current);
    }
}
