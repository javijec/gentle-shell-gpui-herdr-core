//! The active theme's backdrop, drawn behind the panes: a sky gradient, an
//! image (animated GIFs play) and a scene, the theme's own JavaScript run
//! sandboxed in [`scene`]. The panes are glass, so it shows through them at
//! the window's transparency.
//!
//! The scene moves only while the window is focused, and holds still with
//! reduced motion or a speed of 0.

pub mod model;
mod scene;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::shared::themes::Themes;
use crate::shared::ui;
pub use model::{Backdrop, SHARE_EXTENSION};
use scene::{STRIDE, Scene, Setup};

/// The scenes the app ships, adapted from claude-desktop-themes (MIT, see
/// `themes/scenes/LICENSE`) plus the app's own.
const SCENES: &[(&str, &str)] = &[
    ("aurora", include_str!("../../../themes/scenes/aurora.js")),
    ("petals", include_str!("../../../themes/scenes/petals.js")),
    (
        "cloud-drift",
        include_str!("../../../themes/scenes/cloud-drift.js"),
    ),
    (
        "fireflies",
        include_str!("../../../themes/scenes/fireflies.js"),
    ),
    (
        "honu-lagoon",
        include_str!("../../../themes/scenes/honu-lagoon.js"),
    ),
    (
        "jellyfish",
        include_str!("../../../themes/scenes/jellyfish.js"),
    ),
    (
        "koi-pond",
        include_str!("../../../themes/scenes/koi-pond.js"),
    ),
    (
        "lantern-night",
        include_str!("../../../themes/scenes/lantern-night.js"),
    ),
    ("moonlit", include_str!("../../../themes/scenes/moonlit.js")),
    (
        "neon-rain",
        include_str!("../../../themes/scenes/neon-rain.js"),
    ),
    (
        "sakura-dusk",
        include_str!("../../../themes/scenes/sakura-dusk.js"),
    ),
    (
        "snow-cabin",
        include_str!("../../../themes/scenes/snow-cabin.js"),
    ),
    (
        "star-voyage",
        include_str!("../../../themes/scenes/star-voyage.js"),
    ),
];

/// A built-in scene's code by name.
pub fn builtin_scene(name: &str) -> Option<&'static str> {
    SCENES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, code)| *code)
}

/// The names of the built-in scenes.
pub fn builtin_scene_names() -> impl Iterator<Item = &'static str> {
    SCENES.iter().map(|(name, _)| *name)
}

/// The last scene that failed: `(theme, why)`.
static SCENE_ERROR: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

/// Where a failing scene's error is written, so the agent that wrote the
/// scene can read why it doesn't show.
pub fn error_file() -> Option<std::path::PathBuf> {
    crate::shared::themes::app_themes_dir().map(|dir| dir.join("scene-error.txt"))
}

/// A scene of `theme` failed (`Some`) or drew (`None`).
fn report(theme: &str, error: Option<String>) {
    let mut last = SCENE_ERROR.lock().unwrap();
    match error {
        Some(error) => {
            if let Some(path) = error_file() {
                let _ = std::fs::write(
                    path,
                    format!("Theme \"{theme}\": the scene stopped.\n{error}\n"),
                );
            }
            *last = Some((theme.to_string(), error));
        }
        None if last.as_ref().is_some_and(|(t, _)| t == theme) => {
            *last = None;
            if let Some(path) = error_file() {
                let _ = std::fs::remove_file(path);
            }
        }
        None => {}
    }
}

/// Why the scene of the theme called `theme` stopped, if it did.
pub fn scene_error(theme: &str) -> Option<String> {
    SCENE_ERROR
        .lock()
        .unwrap()
        .as_ref()
        .filter(|(t, _)| t == theme)
        .map(|(_, e)| e.clone())
}

/// The scene running for this window, and what it was started from.
#[derive(Default)]
struct Slot {
    running: Option<(Arc<str>, String, Scene)>,
}

/// How much a theme's scene moves, the person's choice in Configuration ›
/// Theme, saved as `backdropMotion` in the app's settings file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Motion {
    /// Up to 24 fps.
    Smooth,
    /// Up to 12 fps: about half the work of Smooth, and the default.
    #[default]
    Calm,
    /// One still frame: no work while it shows.
    Still,
}

impl Motion {
    const KEY: &str = "backdropMotion";
    pub const ALL: [Motion; 3] = [Motion::Smooth, Motion::Calm, Motion::Still];

    pub fn label(self) -> &'static str {
        match self {
            Motion::Smooth => "Smooth",
            Motion::Calm => "Calm",
            Motion::Still => "Still",
        }
    }

    fn floor(self) -> Duration {
        match self {
            Motion::Calm => Duration::from_millis(83),
            Motion::Smooth | Motion::Still => Duration::ZERO,
        }
    }
}

struct MotionChoice(Motion);

impl Global for MotionChoice {}

/// Read the saved motion choice.
pub fn init(cx: &mut App) {
    let saved = crate::shared::themes::store_path()
        .and_then(|path| crate::shared::themes::read_key(&path, Motion::KEY))
        .and_then(|value| {
            let name = value.as_str()?.to_string();
            Motion::ALL
                .into_iter()
                .find(|m| m.label().eq_ignore_ascii_case(&name))
        })
        .unwrap_or_default();
    cx.set_global(MotionChoice(saved));
}

pub fn motion(cx: &App) -> Motion {
    cx.try_global::<MotionChoice>()
        .map_or_else(Motion::default, |m| m.0)
}

/// Choose and save how much scenes move.
pub fn set_motion(motion: Motion, cx: &mut App) {
    cx.set_global(MotionChoice(motion));
    cx.refresh_windows();
    if let Some(path) = crate::shared::themes::store_path() {
        cx.background_executor()
            .spawn(async move {
                let value = serde_json::Value::String(motion.label().to_ascii_lowercase());
                let _ = crate::shared::themes::write_key(&path, Motion::KEY, value);
            })
            .detach();
    }
}

/// The backdrop layer for the window, or `None` when the active theme has none.
pub fn layer(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let theme = cx.global::<Themes>().active_theme().clone();
    let backdrop = theme.background.clone()?;
    let motion = motion(cx);
    let moving = motion != Motion::Still
        && backdrop.speed > 0.
        && window.is_window_active()
        && !cx.reduce_motion();

    // Kept while the layer renders, so the scene stops with the window or the theme.
    let slot = window.use_keyed_state("theme-backdrop", cx, |_, _| Slot::default());
    let scene = backdrop.scene.as_ref().map(|code| {
        let key = format!(
            "{}|{}|{}",
            model::scene_colors(&theme),
            backdrop.cell,
            backdrop.speed
        );
        slot.update(cx, |slot, _| {
            let same = slot
                .running
                .as_ref()
                .is_some_and(|(c, k, _)| Arc::ptr_eq(c, &code.code) && *k == key);
            if !same {
                let started = Scene::start(Setup {
                    theme: theme.name.clone(),
                    code: code.code.clone(),
                    cell: backdrop.cell,
                    speed: backdrop.speed,
                    colors: model::scene_colors(&theme),
                });
                slot.running = Some((code.code.clone(), key, started));
            }
            let (_, _, scene) = slot.running.as_ref().unwrap();
            scene.set_moving(moving);
            scene.set_floor(motion.floor());
            scene.frame()
        })
    });
    if backdrop.scene.is_none() {
        slot.update(cx, |slot, _| slot.running = None);
    }
    let frame = scene.flatten();
    let sky = backdrop.sky.clone();
    let dim = backdrop.dim;
    let veil = backdrop.veil;
    let bg = theme.bg;
    let size_slot = slot.clone();

    let mut layer = div().absolute().inset_0();
    if !sky.is_empty() {
        layer = layer.child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| paint_sky(&sky, bounds, window),
            )
            .absolute()
            .inset_0(),
        );
    }
    if let Some(image) = backdrop.image.clone() {
        layer = layer
            .child(
                img(ImageSource::Image(image))
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
            .when(dim > 0., |d| {
                d.child(div().absolute().inset_0().bg(rgb(bg)).opacity(dim))
            });
    }
    if backdrop.scene.is_some() {
        let frame_size = frame.as_ref().map(|f| f.size);
        layer = layer.child(
            canvas(
                move |bounds, window, cx| {
                    let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    let waiting = frame_size != Some(size);
                    let mut interval = None;
                    if let Some((_, _, scene)) = &size_slot.read(cx).running {
                        scene.set_size(size.0, size.1);
                        interval = Some(scene.interval());
                    }
                    // Redraw at the scene's own pace while it moves, or soon
                    // while a frame for this size is still on its way.
                    if let Some(interval) = interval.filter(|_| moving || waiting) {
                        let delay = if waiting {
                            interval.min(Duration::from_millis(50))
                        } else {
                            interval
                        };
                        ui::request_frame_in(delay, window, cx);
                    }
                },
                move |bounds, _, window, cx| {
                    // Each in one layer: thousands of overlapping shapes
                    // share one draw order instead of each being placed in
                    // GPUI's bounds tree, by far the costliest part otherwise.
                    // Within a layer, shapes keep their order and glyphs draw
                    // over shapes.
                    if let Some(frame) = &frame {
                        window.paint_layer(bounds, |window| {
                            paint_frame(&frame.commands, bounds.origin, window, cx)
                        });
                    }
                    if veil > 0. {
                        window.paint_layer(bounds, |window| paint_veil(bg, veil, bounds, window));
                    }
                },
            )
            .absolute()
            .inset_0(),
        );
    }
    Some(layer.into_any_element())
}

/// Calm the middle of the window, where the conversation is, toward the
/// background, fading in from lively sides.
fn paint_veil(bg: u32, veil: f32, bounds: Bounds<Pixels>, window: &mut Window) {
    let w = bounds.size.width;
    let calm = f64::from(veil) * 0.85;
    // How calm at `u` (0 left, 1 right): clear at the sides, full in the
    // middle, with smoothstep shoulders. A straight ramp meeting the flat
    // middle shows a crease the eye reads as a line, so each shoulder is
    // many short ramps along a curve whose slope fades to zero at both ends.
    let shoulder = |t: f32| {
        let t = t.clamp(0., 1.);
        t * t * (3. - 2. * t)
    };
    let alpha = |u: f32| {
        let rise = shoulder((u - 0.1) / 0.26);
        let fall = shoulder((0.9 - u) / 0.26);
        calm * f64::from(rise.min(fall))
    };
    // Translucent bands meet edge to edge on whole pixels: any overlap
    // would draw twice and show as a darker line.
    let edge = |at: f32| (bounds.origin.x + w * at).round();
    let paint = |from: f32, to: f32, window: &mut Window| {
        let (left, right) = (edge(from), edge(to));
        if right <= left {
            return;
        }
        let band = Bounds::new(
            point(left, bounds.origin.y),
            size(right - left, bounds.size.height),
        );
        let (a, b) = (alpha(from), alpha(to));
        let fill_with: Background = if a == b {
            color(bg as f64, a).into()
        } else {
            linear_gradient(
                90.,
                linear_color_stop(color(bg as f64, a), 0.),
                linear_color_stop(color(bg as f64, b), 1.),
            )
        };
        window.paint_quad(fill(band, fill_with));
    };
    const STEPS: usize = 16;
    for (start, end) in [(0.1, 0.36), (0.64, 0.9)] {
        for i in 0..STEPS {
            let from = start + (end - start) * i as f32 / STEPS as f32;
            let to = start + (end - start) * (i + 1) as f32 / STEPS as f32;
            paint(from, to, window);
        }
    }
    paint(0.36, 0.64, window);
}

fn color(hex: f64, alpha: f64) -> Hsla {
    let c: Hsla = rgb(hex as u32 & 0xffffff).into();
    c.opacity(alpha.clamp(0., 1.) as f32)
}

/// The sky: a vertical gradient through its colors, one band per pair.
fn paint_sky(colors: &[u32], bounds: Bounds<Pixels>, window: &mut Window) {
    if colors.len() == 1 {
        window.paint_quad(fill(bounds, color(colors[0] as f64, 1.)));
        return;
    }
    let bands = colors.len() - 1;
    let height = bounds.size.height / bands as f32;
    for (i, pair) in colors.windows(2).enumerate() {
        let band = Bounds::new(
            point(bounds.origin.x, bounds.origin.y + height * i as f32),
            // A pixel of overlap hides the seams.
            size(bounds.size.width, height + px(1.)),
        );
        let gradient = linear_gradient(
            180.,
            linear_color_stop(color(pair[0] as f64, 1.), 0.),
            linear_color_stop(color(pair[1] as f64, 1.), 1.),
        );
        window.paint_quad(fill(band, gradient));
    }
}

/// A glyph shaped once: its font, id, advance and vertical metrics.
#[derive(Clone, Copy)]
struct Glyph {
    font: FontId,
    id: GlyphId,
    emoji: bool,
    width: f32,
    ascent: f32,
    descent: f32,
}

fn glyph(ch: char, size: f32, bold: bool, window: &mut Window) -> Option<Glyph> {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<(char, u32, bool), Option<Glyph>>> =
            Default::default();
    }
    let key = (ch, (size * 4.).round() as u32, bold);
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        return hit;
    }
    let mut text = [0u8; 4];
    let text = ch.encode_utf8(&mut text);
    let mut font = font(ui::MONO);
    if bold {
        font.weight = FontWeight::BOLD;
    }
    let run = TextRun {
        len: text.len(),
        font,
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let layout = window
        .text_system()
        .layout_line(text, px(size), &[run], None);
    let shaped = layout.runs.first().and_then(|run| {
        let g = run.glyphs.first()?;
        Some(Glyph {
            font: run.font_id,
            id: g.id,
            emoji: g.is_emoji,
            width: f32::from(layout.width),
            ascent: f32::from(layout.ascent),
            descent: f32::from(layout.descent),
        })
    });
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        // Scenes draw a handful of characters; this only guards a runaway one.
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(key, shaped);
    });
    shaped
}

/// Paint a scene's frame (`kit.js` documents the commands).
fn paint_frame(commands: &[f64], origin: Point<Pixels>, window: &mut Window, _: &mut App) {
    let (ox, oy) = (f32::from(origin.x), f32::from(origin.y));
    let at = |x: f64, y: f64| point(px(ox + x as f32), px(oy + y as f32));
    for c in commands.as_chunks::<STRIDE>().0 {
        match c[0] as u32 {
            // rect: x, y, w, h, radius, color, alpha
            1 => {
                if c[7] < 0.005 {
                    continue;
                }
                let bounds = Bounds::new(at(c[1], c[2]), size(px(c[3] as f32), px(c[4] as f32)));
                window.paint_quad(fill(bounds, color(c[6], c[7])).corner_radii(px(c[5] as f32)));
            }
            // ellipse: x, y, rx, ry, color, alpha
            2 => {
                if c[6] < 0.005 {
                    continue;
                }
                let (rx, ry) = (c[3] as f32, c[4] as f32);
                let bounds =
                    Bounds::new(at(c[1] - c[3], c[2] - c[4]), size(px(rx * 2.), px(ry * 2.)));
                window.paint_quad(fill(bounds, color(c[5], c[6])).corner_radii(px(rx.min(ry))));
            }
            // glyph: x (center), y (middle), size, code point, color, alpha, bold
            3 => {
                if c[6] < 0.01 {
                    continue;
                }
                let Some(ch) = char::from_u32(c[4] as u32) else {
                    continue;
                };
                let size = (c[3] as f32).clamp(4., 200.);
                let Some(g) = glyph(ch, size, c[7] != 0., window) else {
                    continue;
                };
                let baseline = at(
                    c[1] - f64::from(g.width) / 2.,
                    c[2] + f64::from(g.ascent - g.descent) / 2.,
                );
                let _ = if g.emoji {
                    window.paint_emoji(baseline, g.font, g.id, px(size))
                } else {
                    window.paint_glyph(baseline, g.font, g.id, px(size), color(c[5], c[6]))
                };
            }
            // glow: x, y, r, color, alpha (no radial gradient in GPUI:
            // stacked discs, densest at the middle)
            4 => {
                // Many faint discs, so the steps between them don't show.
                const RINGS: usize = 28;
                let r = c[3] as f32;
                let each = c[5] * 0.085;
                for ring in 0..RINGS {
                    let rr = r * (1. - ring as f32 / RINGS as f32);
                    let bounds = Bounds::new(
                        at(c[1] - f64::from(rr), c[2] - f64::from(rr)),
                        size(px(rr * 2.), px(rr * 2.)),
                    );
                    window.paint_quad(fill(bounds, color(c[4], each)).corner_radii(px(rr)));
                }
            }
            // gradient: x, y, w, h, angle, from, to, from alpha, to alpha
            5 => {
                let bounds = Bounds::new(at(c[1], c[2]), size(px(c[3] as f32), px(c[4] as f32)));
                let gradient = linear_gradient(
                    c[5] as f32,
                    linear_color_stop(color(c[6], c[8]), 0.),
                    linear_color_stop(color(c[7], c[9]), 1.),
                );
                window.paint_quad(fill(bounds, gradient));
            }
            // line: x1, y1, x2, y2, width, color, alpha
            6 => {
                let mut path = PathBuilder::stroke(px((c[5] as f32).max(0.5)));
                path.move_to(at(c[1], c[2]));
                path.line_to(at(c[3], c[4]));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color(c[6], c[7]));
                }
            }
            _ => {}
        }
    }
}
