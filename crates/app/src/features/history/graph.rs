//! Paints one row of the commit graph: the lanes passing by, the forks and
//! joins as curves, and the node. Paint-only, no input.

use gentle_core::git::log::{Edge, GraphRow};
use gpui_kit::*;

use crate::shared::ui;

/// Horizontal distance between lanes.
const LANE: f32 = 14.;
/// Space before the first lane: lane 0's center.
pub(super) const PAD: f32 = 14.;
const STROKE: f32 = 1.5;
/// Radius the lines keep clear around a node.
const NODE: f32 = 7.;

/// Width of the graph column for `lanes` lanes.
pub(super) fn width(lanes: usize) -> f32 {
    PAD * 2. + LANE * lanes.saturating_sub(1) as f32
}

/// A lane's color: the tip's first-parent line in `info`, the others cycling
/// through the semantic tones.
pub(super) fn lane_color(color: usize) -> Hsla {
    match color % 6 {
        0 => ui::info(),
        1 => ui::warning(),
        2 => ui::danger(),
        3 => ui::success(),
        4 => ui::champagne(),
        _ => ui::petal(),
    }
}

/// What sits on a row's lane.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Node {
    Commit,
    /// HEAD: a ring.
    Head,
    /// Uncommitted changes: a dashed ring.
    Uncommitted,
    /// Nothing: the count bubble is drawn as an element over it.
    None,
}

/// One row's graph. `dashed_top` draws lane 0's top half dashed (the line
/// from the uncommitted row into HEAD); `dashed_bottom` does the same below
/// the uncommitted node.
pub(super) struct RowGraph {
    pub top: Vec<Edge>,
    pub bottom: Vec<Edge>,
    pub lane: usize,
    pub color: usize,
    pub node: Node,
    pub dashed_top: bool,
    pub dashed_bottom: bool,
}

impl RowGraph {
    pub fn commit(row: &GraphRow, head: bool, dashed_top: bool) -> Self {
        Self {
            top: row.top.clone(),
            bottom: row.bottom.clone(),
            lane: row.lane,
            color: row.color,
            node: if head { Node::Head } else { Node::Commit },
            dashed_top,
            dashed_bottom: false,
        }
    }

    pub fn uncommitted() -> Self {
        Self {
            top: Vec::new(),
            bottom: Vec::new(),
            lane: 0,
            color: 0,
            node: Node::Uncommitted,
            dashed_top: false,
            dashed_bottom: true,
        }
    }

    /// Lane 0 coming down into the "Earlier history" bubble.
    pub fn earlier() -> Self {
        Self {
            top: vec![Edge {
                from: 0,
                to: 0,
                color: 0,
            }],
            bottom: Vec::new(),
            lane: 0,
            color: 0,
            node: Node::None,
            dashed_top: false,
            dashed_bottom: false,
        }
    }

    pub fn element(self) -> impl IntoElement {
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| self.paint(bounds, window),
        )
        .size_full()
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let x = |lane: usize| bounds.origin.x + px(PAD + LANE * lane as f32);
        let top = bounds.origin.y;
        let mid = bounds.origin.y + bounds.size.height / 2.;
        let bottom = bounds.origin.y + bounds.size.height;
        // Lines stop short of the node, so a ring stays hollow on any row
        // background (hover, selection) without masking.
        let gap = px(if self.node == Node::None { 0. } else { NODE });
        let into = |lane: usize| if lane == self.lane { mid - gap } else { mid };
        let out_of = |lane: usize| if lane == self.lane { mid + gap } else { mid };

        for edge in &self.top {
            let dashed = self.dashed_top && edge.from == 0 && edge.to == 0;
            let end = point(x(edge.to), into(edge.to));
            line(window, point(x(edge.from), top), end, edge.color, dashed);
        }
        if self.dashed_top && !self.top.iter().any(|e| e.to == 0) {
            line(window, point(x(0), top), point(x(0), into(0)), 0, true);
        }
        for edge in &self.bottom {
            let start = point(x(edge.from), out_of(edge.from));
            line(window, start, point(x(edge.to), bottom), edge.color, false);
        }
        if self.dashed_bottom {
            line(window, point(x(0), out_of(0)), point(x(0), bottom), 0, true);
        }

        let color = lane_color(self.color);
        let center = point(x(self.lane), mid);
        match self.node {
            Node::Commit => {
                circle(window, center, NODE - 1.5, color.opacity(0.22), None);
                circle(window, center, 3.5, color, None);
            }
            Node::Head => circle(
                window,
                center,
                NODE - 1.,
                transparent_black(),
                Some((color, BorderStyle::Solid)),
            ),
            Node::Uncommitted => circle(
                window,
                center,
                NODE - 1.,
                transparent_black(),
                Some((color.opacity(0.8), BorderStyle::Dashed)),
            ),
            Node::None => {}
        }
    }
}

/// A straight line between equal lanes, else an S-curve that leaves and
/// arrives vertically.
fn line(window: &mut Window, from: Point<Pixels>, to: Point<Pixels>, color: usize, dashed: bool) {
    let mut path = PathBuilder::stroke(px(STROKE));
    if dashed {
        path = path.dash_array(&[px(3.), px(3.)]);
    }
    path.move_to(from);
    if from.x == to.x {
        path.line_to(to);
    } else {
        let ym = (from.y + to.y) / 2.;
        path.cubic_bezier_to(to, point(from.x, ym), point(to.x, ym));
    }
    if let Ok(path) = path.build() {
        let color = lane_color(color);
        window.paint_path(path, if dashed { color.opacity(0.8) } else { color });
    }
}

fn circle(
    window: &mut Window,
    center: Point<Pixels>,
    radius: f32,
    fill: Hsla,
    border: Option<(Hsla, BorderStyle)>,
) {
    let r = px(radius);
    let bounds = Bounds::new(point(center.x - r, center.y - r), size(r * 2., r * 2.));
    let (width, color, style) = match border {
        Some((color, style)) => (px(2.), color, style),
        None => (px(0.), transparent_black(), BorderStyle::Solid),
    };
    window.paint_quad(quad(bounds, r, fill, width, color, style));
}
