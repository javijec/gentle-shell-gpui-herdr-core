//! The active theme's backdrop, drawn behind the panes: a sky gradient, an
//! image (animated GIFs play) and a scene, the theme's own JavaScript run
//! sandboxed in [`scene`]. The panes are glass, so it shows through them at
//! the window's transparency.
//!
//! The scene moves only while the window is focused, and holds still with
//! reduced motion or a speed of 0.

mod effects;
pub mod model;
pub mod preview;
mod scene;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::platform::power::{self, Power};
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

/// The scene running for this window, and what it was started from; and
/// the latest frame's effect patches as images (dropped from GPUI's atlas
/// when the next frame replaces them).
#[derive(Default)]
struct Slot {
    running: Option<(Arc<str>, String, Scene)>,
    patches: Option<(usize, Vec<Arc<RenderImage>>)>,
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

/// The machine's power state, polled every [`POWER_POLL`].
struct PowerNow(Power);

impl Global for PowerNow {}

/// How often the power state is read again.
const POWER_POLL: Duration = Duration::from_secs(20);

/// Read the saved motion choice, and keep following the power state.
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
    cx.set_global(PowerNow(power::current()));
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POWER_POLL).await;
            let now = cx
                .background_executor()
                .spawn(async { power::current() })
                .await;
            cx.update(|cx| {
                if cx.global::<PowerNow>().0 != now {
                    cx.set_global(PowerNow(now));
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}

/// How much scenes move by the person's choice.
pub fn motion(cx: &App) -> Motion {
    cx.try_global::<MotionChoice>()
        .map_or_else(Motion::default, |m| m.0)
}

/// How much scenes move now: the person's choice, eased by the power state.
/// On battery Smooth slows to Calm; saving power (Low Power Mode, battery
/// saver) holds every scene still.
pub fn motion_now(cx: &App) -> Motion {
    let power = cx
        .try_global::<PowerNow>()
        .map_or_else(Power::default, |p| p.0);
    eased(motion(cx), power)
}

fn eased(motion: Motion, power: Power) -> Motion {
    match (motion, power) {
        (_, Power::Saving) => Motion::Still,
        (Motion::Smooth, Power::Battery) => Motion::Calm,
        (motion, _) => motion,
    }
}

/// Why scenes move less than chosen right now, if they do.
pub fn power_note(cx: &App) -> Option<&'static str> {
    let power = cx
        .try_global::<PowerNow>()
        .map_or_else(Power::default, |p| p.0);
    (motion_now(cx) != motion(cx)).then_some(match power {
        Power::Saving => "Saving power: scenes hold still.",
        _ => "On battery: scenes move at Calm.",
    })
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
    let motion = motion_now(cx);
    let moving = motion != Motion::Still
        && backdrop.speed > 0.
        && window.is_window_active()
        && !cx.reduce_motion();

    // Kept while the layer renders, so the scene stops with the window or the theme.
    let slot = window.use_keyed_state("theme-backdrop", cx, |_, _| Slot::default());
    // A camera or effects move the image on the scene's clock, so they run
    // a scene that draws nothing when the theme has none.
    let natural = backdrop.image.as_ref().and(backdrop.image_size);
    let code = backdrop
        .scene
        .as_ref()
        .map(|s| s.code.clone())
        .or_else(|| backdrop.moves().then(|| STILL.clone()));
    let scene = code.as_ref().map(|code| {
        let key = format!(
            "{}|{}|{}|{:?}|{:?}|{:?}",
            model::scene_colors(&theme),
            backdrop.cell,
            backdrop.speed,
            natural,
            backdrop.camera,
            backdrop.effects
        );
        slot.update(cx, |slot, _| {
            let same = slot
                .running
                .as_ref()
                .is_some_and(|(c, k, _)| Arc::ptr_eq(c, code) && *k == key);
            if !same {
                let started = Scene::start(Setup {
                    theme: theme.name.clone(),
                    code: code.clone(),
                    cell: backdrop.cell,
                    speed: backdrop.speed,
                    colors: model::scene_colors(&theme),
                    image: natural.map(|size| (size, backdrop.camera)),
                    effects: backdrop
                        .image
                        .clone()
                        .filter(|_| !backdrop.effects.is_empty())
                        .map(|image| (image, backdrop.effects.clone())),
                });
                slot.running = Some((code.clone(), key, started));
            }
            let (_, _, scene) = slot.running.as_ref().unwrap();
            scene.set_moving(moving);
            scene.set_floor(motion.floor());
            scene.frame()
        })
    });
    if code.is_none() {
        slot.update(cx, |slot, _| slot.running = None);
    }
    let frame = scene.flatten();
    let sky = backdrop.sky.clone();
    let dim = backdrop.dim;
    let veil = backdrop.veil;
    let bg = theme.bg;
    let size_slot = slot.clone();
    let patch_slot = slot.clone();
    // With a scene running, the image is painted where each frame puts it,
    // so what the scene draws on it stays on it as the camera moves.
    let framed_image = backdrop
        .image
        .clone()
        .filter(|_| code.is_some() && natural.is_some());

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
    if let Some(image) = backdrop.image.clone().filter(|_| framed_image.is_none()) {
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
    if code.is_some() {
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
                    let image = framed_image?.use_render_image(window, cx)?;
                    let (index, next) = gif_frame(&image);
                    if let Some(next) = next.filter(|_| moving) {
                        ui::request_frame_in(next, window, cx);
                    }
                    Some((image, index))
                },
                move |bounds, image, window, cx| {
                    if let Some((image, index)) = image {
                        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                        let [x, y, w, h] = frame
                            .as_ref()
                            .filter(|f| f.size == size)
                            .and_then(|f| f.image)
                            .or_else(|| natural.map(|n| model::image_rect(size, n, None, 0.)))
                            .unwrap_or([0., 0., size.0, size.1]);
                        let at = Bounds::new(
                            point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
                            gpui_kit::size(px(w), px(h)),
                        );
                        let _ =
                            window.paint_image(bounds, at, Corners::default(), image, index, false);
                        if let Some(frame) = frame.as_ref().filter(|f| f.size == size) {
                            paint_patches(frame, at, bounds, &patch_slot, window, cx);
                        }
                        if dim > 0. {
                            window.paint_quad(fill(bounds, color(f64::from(bg), f64::from(dim))));
                        }
                    }
                    // Each in one layer: thousands of overlapping shapes
                    // share one draw order instead of each being placed in
                    // GPUI's bounds tree, by far the costliest part otherwise.
                    // Within a layer, shapes keep their order and glyphs draw
                    // over shapes.
                    if let Some(frame) = &frame {
                        window.paint_layer(bounds, |window| {
                            paint_frame(&frame.commands, &frame.points, bounds.origin, window, cx)
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

/// An empty scene: it runs the camera and effects over an image when the
/// theme has no scene of its own.
static STILL: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("SCENES.still = {};"));

/// Paint `frame`'s effect patches over the picture drawn at `picture`. Each
/// frame's patches become images once; the previous frame's are dropped
/// from GPUI's atlas, so a moving effect doesn't fill it.
fn paint_patches(
    frame: &Arc<scene::Frame>,
    picture: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    slot: &Entity<Slot>,
    window: &mut Window,
    cx: &mut App,
) {
    if frame.patches.is_empty() {
        return;
    }
    let key = Arc::as_ptr(frame) as usize;
    let (images, stale) = slot.update(cx, |slot, _| {
        if let Some((at, images)) = &slot.patches
            && *at == key
        {
            return (images.clone(), Vec::new());
        }
        let images: Vec<Arc<RenderImage>> = frame
            .patches
            .iter()
            .filter_map(|patch| {
                let buffer = image::RgbaImage::from_raw(
                    patch.width,
                    patch.height,
                    patch.bgra.as_ref().clone(),
                )?;
                Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
            })
            .collect();
        let stale = slot
            .patches
            .replace((key, images.clone()))
            .map(|(_, old)| old)
            .unwrap_or_default();
        (images, stale)
    });
    for old in stale {
        let _ = window.drop_image(old);
    }
    let (x, y) = (f32::from(picture.origin.x), f32::from(picture.origin.y));
    let (w, h) = (
        f32::from(picture.size.width),
        f32::from(picture.size.height),
    );
    for (patch, image) in frame.patches.iter().zip(images) {
        let [u0, v0, u1, v1] = patch.rect;
        let at = Bounds::new(
            point(px(x + u0 * w), px(y + v0 * h)),
            gpui_kit::size(px((u1 - u0) * w), px((v1 - v0) * h)),
        );
        let _ = window.paint_image(clip, at, Corners::default(), image, 0, false);
    }
}

/// The frame of an animated image (a GIF) to show now, by the wall clock,
/// and how long until the next one; a still image is frame 0, for good.
fn gif_frame(image: &RenderImage) -> (usize, Option<Duration>) {
    let count = image.frame_count();
    if count < 2 {
        return (0, None);
    }
    let delays: Vec<u128> = (0..count)
        .map(|i| Duration::from(image.delay(i)).as_millis().max(20))
        .collect();
    let total: u128 = delays.iter().sum();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
        % total;
    let mut start = 0;
    for (i, delay) in delays.iter().enumerate() {
        if now < start + delay {
            let left = (start + delay - now) as u64;
            return (i, Some(Duration::from_millis(left)));
        }
        start += delay;
    }
    (0, None)
}

/// How calm the veil is at `u` (0 left, 1 right): clear at the sides, full
/// in the middle, with smoothstep shoulders. A straight ramp meeting the
/// flat middle shows a crease the eye reads as a line, so each shoulder is
/// many short ramps along a curve whose slope fades to zero at both ends.
fn veil_alpha(veil: f32, u: f32) -> f64 {
    let shoulder = |t: f32| {
        let t = t.clamp(0., 1.);
        t * t * (3. - 2. * t)
    };
    let rise = shoulder((u - 0.1) / 0.26);
    let fall = shoulder((0.9 - u) / 0.26);
    f64::from(veil) * 0.85 * f64::from(rise.min(fall))
}

/// Calm the middle of the window, where the conversation is, toward the
/// background, fading in from lively sides.
fn paint_veil(bg: u32, veil: f32, bounds: Bounds<Pixels>, window: &mut Window) {
    let w = bounds.size.width;
    let alpha = |u: f32| veil_alpha(veil, u);
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

/// The subpaths of a path command's `count` points from `start` (`kit.js`:
/// each run of x, y pairs ends with a NaN pair, `1` when it closes), or
/// nothing when the command points outside `points`.
fn subpaths(points: &[f64], start: f64, count: f64) -> impl Iterator<Item = (&[[f64; 2]], bool)> {
    let pairs = points.as_chunks::<2>().0;
    let (start, count) = (start as usize, count as usize);
    let run = pairs.get(start..start.saturating_add(count)).unwrap_or(&[]);
    run.split_inclusive(|p| p[0].is_nan())
        .map(|run| match run.split_last() {
            Some((last, body)) if last[0].is_nan() => (body, last[1] != 0.),
            _ => (run, false),
        })
}

/// Paint a scene's frame (`kit.js` documents the commands).
fn paint_frame(
    commands: &[f64],
    points: &[f64],
    origin: Point<Pixels>,
    window: &mut Window,
    _: &mut App,
) {
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
            // path: start, count, stroke width (0 fills), color, alpha
            // path gradient: start, count, angle, from, to, from alpha, to alpha
            7 | 8 => {
                let gradient = c[0] as u32 == 8;
                let width = if gradient { 0. } else { c[3] as f32 };
                if !gradient && c[5] < 0.005 {
                    continue;
                }
                let mut path = if width > 0. {
                    PathBuilder::stroke(px(width))
                } else {
                    PathBuilder::fill()
                };
                for (run, closed) in subpaths(points, c[1], c[2]) {
                    let [first, rest @ ..] = run else { continue };
                    if rest.is_empty() {
                        continue;
                    }
                    path.move_to(at(first[0], first[1]));
                    for p in rest {
                        path.line_to(at(p[0], p[1]));
                    }
                    if closed || width == 0. {
                        path.close();
                    }
                }
                let Ok(path) = path.build() else { continue };
                if gradient {
                    window.paint_path(
                        path,
                        linear_gradient(
                            c[3] as f32,
                            linear_color_stop(color(c[4], c[6]), 0.),
                            linear_color_stop(color(c[5], c[7]), 1.),
                        ),
                    );
                } else {
                    window.paint_path(path, color(c[4], c[5]));
                }
            }
            _ => {}
        }
    }
}
