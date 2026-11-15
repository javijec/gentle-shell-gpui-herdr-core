//! Pane size math: minimum pane sizes and split fits, the tiling direction
//! for a new pane, and where a dragged pane lands.

use super::*;

/// Smallest useful pane; splits and divider drags stop here.
pub(super) const MIN_PANE_WIDTH: f32 = 320.;
pub(super) const MIN_PANE_HEIGHT: f32 = 200.;
/// Space a divider takes between two panes.
pub(super) const DIVIDER: f32 = 6.;

/// The side docks are independent of the central split-pane dividers.
pub(super) const MIN_DOCK_WIDTH: f32 = 180.;
pub(super) const MAX_DOCK_WIDTH: f32 = 640.;
pub(super) const DOCK_HANDLE_WIDTH: f32 = 6.;
const DOCK_PADDING: f32 = 12.;
const DOCK_GAP: f32 = 6.;
const MIN_CENTER_WIDTH: f32 = 320.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DockSide {
    Machines,
    Files,
}

#[derive(Clone)]
pub(super) struct DockResizeDrag {
    pub side: DockSide,
}

/// Keep the central workspace usable when the window becomes narrow. At
/// ordinary sizes, preserve the exact user-selected widths. There are two
/// gaps and one drag handle per visible dock.
pub(super) fn fit_dock_widths(
    viewport: f32,
    machines: Option<f32>,
    files: Option<f32>,
) -> (f32, f32) {
    let sanitise = |width: Option<f32>| match width {
        Some(value) if value.is_finite() => value.clamp(MIN_DOCK_WIDTH, MAX_DOCK_WIDTH),
        Some(_) => MIN_DOCK_WIDTH,
        None => 0.,
    };
    let left = sanitise(machines);
    let right = sanitise(files);
    let count = f32::from(u8::from(machines.is_some()) + u8::from(files.is_some()));
    let chrome = DOCK_PADDING + count * (DOCK_HANDLE_WIDTH + 2. * DOCK_GAP);
    let center = MIN_CENTER_WIDTH.min((viewport.max(0.) * 0.5).max(0.));
    let available = (viewport - chrome - center).max(0.);
    let total = left + right;
    if total <= available || total == 0. {
        (left, right)
    } else {
        let scale = available / total;
        (left * scale, right * scale)
    }
}

/// Bound a drag by the other dock's rendered size and a minimum center pane.
pub(super) fn resized_dock_width(
    viewport: f32,
    machines: f32,
    files: f32,
    side: DockSide,
    pointer: f32,
) -> f32 {
    let count = f32::from(u8::from(machines > 0.) + u8::from(files > 0.));
    let chrome = DOCK_PADDING + count * (DOCK_HANDLE_WIDTH + 2. * DOCK_GAP);
    let center = MIN_CENTER_WIDTH.min(viewport.max(0.) * 0.5);
    let remaining = (viewport - chrome - center).max(0.);
    let other = match side {
        DockSide::Machines => files,
        DockSide::Files => machines,
    };
    let maximum = (remaining - other).max(0.).min(MAX_DOCK_WIDTH);
    let minimum = MIN_DOCK_WIDTH.min(maximum);
    pointer.clamp(minimum, maximum)
}

impl Workspace {
    /// Both Files variants share a custom width; before the first drag their
    /// original local (260px) and remote (330px) defaults remain unchanged.
    pub(super) fn dock_widths(
        &self,
        viewport: f32,
        selected_remote: bool,
        machines_visible: bool,
        files_visible: bool,
    ) -> (f32, f32) {
        fit_dock_widths(
            viewport,
            machines_visible.then_some(self.machines_dock_width),
            files_visible.then_some(self.files_dock_width.unwrap_or(
                if selected_remote { SIDEBAR_WIDTH + 70. } else { SIDEBAR_WIDTH },
            )),
        )
    }
}

/// Whether a `w`×`h` pane split along `axis` leaves both halves usable.
pub(super) fn fits_split(axis: SplitAxis, w: f32, h: f32) -> bool {
    match axis {
        SplitAxis::Horizontal => (w - DIVIDER) / 2. >= MIN_PANE_WIDTH,
        SplitAxis::Vertical => (h - DIVIDER) / 2. >= MIN_PANE_HEIGHT,
    }
}

/// Where a new pane goes when tiling (Hyprland's dwindle): split `w`×`h`
/// along its longer side, else the other one if only that fits. `None` when
/// neither half would be usable.
pub(super) fn tile_axis(w: f32, h: f32) -> Option<SplitAxis> {
    let (long, short) = if w >= h {
        (SplitAxis::Horizontal, SplitAxis::Vertical)
    } else {
        (SplitAxis::Vertical, SplitAxis::Horizontal)
    };
    [long, short]
        .into_iter()
        .find(|axis| fits_split(*axis, w, h))
}

/// Where a pane dragged over another one lands: near an edge it moves to
/// that side of the target, in the middle the two swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropZone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

impl DropZone {
    /// The zone under the pointer at `x`, `y` (0..1 across the target pane).
    pub(super) fn at(x: f32, y: f32) -> Self {
        let (dx, dy) = (x - 0.5, y - 0.5);
        if dx.abs() < 0.2 && dy.abs() < 0.2 {
            DropZone::Center
        } else if dx.abs() >= dy.abs() {
            if dx < 0. {
                DropZone::Left
            } else {
                DropZone::Right
            }
        } else if dy < 0. {
            DropZone::Top
        } else {
            DropZone::Bottom
        }
    }

    /// The split that puts the dragged pane on this side (axis, before the
    /// target); `None` for the middle, which swaps.
    pub(super) fn placement(self) -> Option<(SplitAxis, bool)> {
        match self {
            DropZone::Left => Some((SplitAxis::Horizontal, true)),
            DropZone::Right => Some((SplitAxis::Horizontal, false)),
            DropZone::Top => Some((SplitAxis::Vertical, true)),
            DropZone::Bottom => Some((SplitAxis::Vertical, false)),
            DropZone::Center => None,
        }
    }
}

/// Each leaf's size when `node` fills `w`×`h`.
pub(super) fn leaf_sizes(
    node: &PaneTree<PaneId>,
    w: f32,
    h: f32,
    out: &mut Vec<(PaneId, f32, f32)>,
) {
    match node {
        PaneTree::Leaf(id) => out.push((*id, w, h)),
        PaneTree::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            if *axis == SplitAxis::Horizontal {
                let inner = w - DIVIDER;
                leaf_sizes(first, inner * ratio, h, out);
                leaf_sizes(second, inner * (1. - ratio), h, out);
            } else {
                let inner = h - DIVIDER;
                leaf_sizes(first, w, inner * ratio, out);
                leaf_sizes(second, w, inner * (1. - ratio), out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DockSide, DropZone, SplitAxis, fit_dock_widths, resized_dock_width, tile_axis,
    };

    #[test]
    fn dock_widths_keep_the_center_visible_and_restore_full_size() {
        assert_eq!(fit_dock_widths(1400., Some(320.), Some(410.)), (320., 410.));
        let (left, right) = fit_dock_widths(780., Some(320.), Some(410.));
        assert!(left > 0. && right > 0.);
        // 12px outer padding, 2x (6px handle + 2x6px gap).
        assert!(left + right + 48. + 320. <= 780.01);
        assert_eq!(fit_dock_widths(1400., None, Some(400.)), (0., 400.));
        assert_eq!(fit_dock_widths(1400., Some(400.), None), (400., 0.));
        assert_eq!(fit_dock_widths(1400., Some(f32::NAN), None).0, 180.);
    }

    #[test]
    fn each_dock_drag_is_bounded_by_the_other_and_center() {
        assert_eq!(resized_dock_width(
            1200., 260., 330., DockSide::Machines, 800.,
        ), 502.);
        assert_eq!(resized_dock_width(
            1200., 260., 330., DockSide::Files, 50.,
        ), 180.);
    }

    #[test]
    fn tiling_splits_the_longer_side_while_it_fits() {
        assert_eq!(tile_axis(1200., 800.), Some(SplitAxis::Horizontal));
        assert_eq!(tile_axis(600., 800.), Some(SplitAxis::Vertical));
        // Wide but too narrow to halve: stack instead.
        assert_eq!(tile_axis(640., 500.), Some(SplitAxis::Vertical));
        // Neither half would be usable.
        assert_eq!(tile_axis(500., 300.), None);
    }

    #[test]
    fn drop_zones_follow_the_nearest_edge_and_swap_in_the_middle() {
        assert_eq!(DropZone::at(0.5, 0.5), DropZone::Center);
        assert_eq!(DropZone::at(0.05, 0.5), DropZone::Left);
        assert_eq!(DropZone::at(0.95, 0.4), DropZone::Right);
        assert_eq!(DropZone::at(0.5, 0.1), DropZone::Top);
        assert_eq!(DropZone::at(0.6, 0.9), DropZone::Bottom);
        assert_eq!(
            DropZone::Left.placement(),
            Some((SplitAxis::Horizontal, true))
        );
        assert_eq!(
            DropZone::Bottom.placement(),
            Some((SplitAxis::Vertical, false))
        );
        assert_eq!(DropZone::Center.placement(), None);
    }
}
