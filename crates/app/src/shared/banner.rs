//! Gentle Shell's startup banner: the braille rose and the pixel "Gentle Shell"
//! wordmark, drawn as real pixels instead of terminal glyphs. The art comes
//! from the brand assets `gentle_rose.txt` and `gentle_shell_combo.txt` (and
//! their `_color` versions for the rose's shading and the wordmark's two
//! tones); palettes and `banner.json` semantics from gentle-pi
//! `extensions/startup-banner.ts`.

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use gpui_kit::*;

use crate::shared::ui;

const TEXT_LOGO: &[&str] = &[
    "              ▄▄▄▀▀▀▀▀██                              ▄▄▀▄▄          ▄▄█▀▀▀██         ▄▄▀▄▄           ▄▄▀▄▄    ▄▄▀▄▄",
    "          ▄▄█▀▀▒▒▒▒▒▄▄█▀▒                 ▄██     ▄▄█▀█▄█▀▒▒       ▄█▀▀▒▒▒▄█▀▒    ▄▄█▀█▄█▀▒▒      ▄▄█▀█▄█▀▒▄▄█▀█▄█▀▒▒",
    "      ▄▄██▀▒▒▒▒▒▄▄▄▀▀▒▒▒▒           ▀▀▀▀██▀▀▀▀▀███▀█▄▀▀▒▒▒▒      ██▄▄▒▒▒▒▒▒▒   ▄██▀█▄▀▀▒▒▒▒    ▄██▀█▄▀▀▒▄██▀█▄▀▀▒▒▒▒",
    "    ▄██▀▒▒▒▒     ▒▒▄▄█ ▄▄▄▀██ █▄▀█▄  ▄██▀▒▒▒▒▄██▀▀▀▄▄▄▀██          ▀▀▀██▄▄   ▄███▀▀▒▒▄▄▄▀██  ▄██▀▀▀▒▒ ▄██▀▀▀▒▒",
    "   ██▀▒▒▒     ▄▄▄███▀▄██▄▄█▀ ██▒▄█▀▄██▀▒▒▒▄▄██▀▒▒▄██▄▄█▀▒       ▄▄     ▀██ ▄██▀▒██▒▄██▄▄█▀▄▄██▀▒▒▒ ▄▄██▀▒▒▄▄▄",
    "    ▀█▄▄▄▄▄▀▀▀█▄▄███▄█▀▀▄▄▄▀██▒▒█▄▀▒██▄▄▀▀▀▒▀█▄▄██▀▀▄▄▄▀▀        ▀█▄▄▄▄█▀▀█▀▀▒▒██▄▄█▀▀▄▄▄▀▀▒▀█▄▄▀▀▀▒▀█▄▄▀▀▀▒",
    "     ▒▄▄▄█▀▀▀█▄█▀▀▒▒▒▒▒  ▒▒▒▒▒ ▒▒▒▒▒ ▒▒ ▒▒▒▒▒▒▒ ▒▒▒▒▒▒ ▒▒▒▒        ▒▀▀▀▀▒▒▒▒  ▒▒▒▒▒▒ ▒▒▒▒▒ ▒▒▒▒▒▒  ▒▒▒▒▒▒▒▒",
    "  ▄▄▀▀ ▒▒▒▒▄██▀▒▒▒▒",
    "▄█ ▒▒▒▄▄██▀▀▒▒▒▒",
    " ▀▀▀▀▀▀▒▒▒▒▒▒",
    "  ▒▒▒▒▒▒",
];

const ROSE: &[&str] = &[
    "            ⣠⣿⣿⣶⣶⣤⣤⣤⣤⣤⣤⣤⣀    ⢀⡀",
    "         ⢀⣴⣾⣿⣿⠟⠝⢙⣙⣙⣙⣙⢙⣿⣷⢿⢿⢿⢿⢿⣿⣷⣄⡀",
    "      ⣀⣠⣴⣿⣿⣿⠟⢡⣶⡿⠟⢟⡽⣿⣿⣿⣿⡟⢶⣤⣤⣄⣁⠁⠙⢿⣿⣦",
    "   ⢀⣴⣿⣿⠅⣿⣿⣿⠅⢀⣿⠏ ⢰⡏⢹⢯⡤⢖⣯⣍⠻⡍⢻⡌⢽⣷⠄ ⢿⣿⡷⢶⣤⣤",
    "⢀⣀⣴⣿⣿⣿⠟ ⠹⣿⣿  ⢻⣇ ⠘⠻⣦⣈⢙⣻⣿⣷⣾⡥⠜⢁⣼⠟⠁⢴⣿⡿⠁⢽⣿⣿⣄",
    "⢻⣿⢿⣿⣿⣟⠅  ⠘⢿⣧ ⠘⠙⢦⣀  ⠙⠳⢤  ⢀⡴⠞⠛⠝⢀⣴⣿⠟  ⢽⣿⣿⢿⡆",
    " ⣿⢝⢿⣿⣧⠁   ⢸⠍⠳⣄  ⠉⠓⠦⢄⢀⣀⣁⣤⣥⣤⣤⣶⠿⢿⡟⠁   ⣽⣿⢿⣽⡇",
    " ⢿⣟⠝⢿⣿⣷⣄  ⠝⠕⠅⠈⠂ ⢀⡤⠖⠛⠉⠉⡁⣀   ⢀⠅⢽⠇  ⢀⣽⣿⣿⣿⠏",
    " ⠈⠻⢷⣝⣝⣿⢿⢷⣤⣄    ⠴⠉⠄⢔⢴⢾⢿⢿⢿⢗⠔⠔⠄⠁ ⢁⢄⣵⣿⡿⠟⢉⢽⣷⡄",
    "    ⠈⠉⠉⢿⡛⠿⢿⣿⣶⣤⣀⡁  ⠁⠁⠙⠅⠙⠁⠁⠁ ⣀⣴⣾⢿⠿⠛⠉ ⢤⣽⣿⣿⣇",
    "       ⠈⢿⣧⡄ ⠁⠉⠛⠿⠿⢷⣶⣤⣀   ⢀⡴⠛⠉⠁ ⠁ ⢀⣴⢿⢿⣽⠟⠉⠁",
    "         ⠙⠻⣷⣥⣄⢀    ⠈⠙⢷⡄ ⠁ ⢀⢄⣴⣴⢵⠿⢟⣭⣽⠟⠁",
    "            ⢉⢙⣛⣿⠷⢤⣄⡀  ⢿⢠⠔⢓⣛⠿⠿⠷⠷⠟⠛⠉⠁",
    "            ⠈⠙⠛⠛⠓⠒⢖⠙⠷⡄⢈⡴⠟⢅⢴⣶⣤⣤⠄",
    "                   ⢷⡄⠱⢍⡤⠐⠉⠉⠉⠉",
    "                   ⠈⣷ ⠍",
    "                    ⣿⢄",
];

/// Each rose cell's shade, `0` darkest to `9` lightest, from
/// `gentle_rose_color.txt`: the cell's lightness over the base color's,
/// `0.55 + 0.075 × shade` ([`shade_factor`]).
const ROSE_SHADE: &[&str] = &[
    "            7767687656668    66",
    "         678755455455555564566796",
    "      6757654566556666656555433478",
    "   687536652366 665666665565563 476675",
    "3887663 465  65 56666666666666045563479",
    "8656642  565 5666  666  656536566  35477",
    " 835643   4566  6665666566655556   45367",
    " 7435643  22266 66666653   0026  335556",
    " 3866545555    26033454333300 2344555575",
    "    666665655662  03222300 6565566 24468",
    "       6755 356655555   66563 0 34347886",
    "         776543    3656 3 33444344686",
    "            76765553  5335555556780",
    "            6965564445565346896",
    "                   5532433565",
    "                   36 2",
    "                    62",
];

/// Columns of space between the rose and the wordmark in the combo asset.
const GAP: f32 = 5.0;

/// A [`ROSE_SHADE`] digit as a lightness multiplier.
fn shade_factor(shade: char) -> f32 {
    0.55 + 0.075 * shade.to_digit(10).unwrap_or(6) as f32
}

/// One pixel of the wordmark: half-block cells are 1×2 pixels; `▒` is the
/// darker drop shadow.
#[derive(Clone, Copy)]
struct Pixel {
    x: u16,
    y: u16,
    shadow: bool,
    /// When the pen reaches it, 0..=1 of the writing ([`pen_times`]).
    at: f32,
}

struct Art {
    logo: Vec<Pixel>,
    logo_size: (u16, u16),
    rose: Vec<(u16, u16)>,
    /// [`shade_factor`] of each `rose` dot, in the same order.
    rose_shade: Vec<f32>,
    /// When each `rose` dot opens, 0..=1 of the bloom ([`bloom_times`]).
    rose_at: Vec<f32>,
    rose_size: (u16, u16),
}

static ART: LazyLock<Art> = LazyLock::new(|| {
    let mut cells = Vec::new();
    let mut logo_w = 0;
    for (row, line) in TEXT_LOGO.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            let (x, top, bottom) = (col as u16, row as u16 * 2, row as u16 * 2 + 1);
            let pixels: &[(u16, bool)] = match ch {
                '▀' => &[(top, false)],
                '▄' => &[(bottom, false)],
                '█' => &[(top, false), (bottom, false)],
                '▒' => &[(top, true), (bottom, true)],
                _ => &[],
            };
            for &(y, shadow) in pixels {
                cells.push((x, y, shadow));
                logo_w = logo_w.max(x + 1);
            }
        }
    }
    let ink: Vec<(u16, u16)> = cells
        .iter()
        .filter(|c| !c.2)
        .map(|&(x, y, _)| (x, y))
        .collect();
    let all: Vec<(u16, u16)> = cells.iter().map(|&(x, y, _)| (x, y)).collect();
    let times = pen_times(&ink, &all);
    let mut logo: Vec<Pixel> = cells
        .iter()
        .zip(times)
        .map(|(&(x, y, shadow), at)| Pixel { x, y, shadow, at })
        .collect();
    // Shadows sit behind the ink.
    logo.sort_by_key(|p| !p.shadow);

    // Braille U+2800: bits 0-2 and 6 are the left column (rows 0-3), 3-5 and 7 the right.
    const DOTS: [(u16, u16); 8] = [
        (0, 0),
        (0, 1),
        (0, 2),
        (1, 0),
        (1, 1),
        (1, 2),
        (0, 3),
        (1, 3),
    ];
    let mut rose = Vec::new();
    let mut rose_shade = Vec::new();
    let mut rose_w = 0;
    for (row, (line, shades)) in ROSE.iter().zip(ROSE_SHADE).enumerate() {
        let shades = shades.chars().chain(std::iter::repeat(' '));
        for (col, (ch, shade)) in line.chars().zip(shades).enumerate() {
            let code = ch as u32;
            if !(0x2800..=0x28FF).contains(&code) {
                continue;
            }
            let bits = code - 0x2800;
            for (bit, (dx, dy)) in DOTS.iter().enumerate() {
                if bits & (1 << bit) != 0 {
                    rose.push((col as u16 * 2 + dx, row as u16 * 4 + dy));
                    rose_shade.push(shade_factor(shade));
                    rose_w = rose_w.max(col as u16 * 2 + dx + 1);
                }
            }
        }
    }
    let rose_size = (rose_w, ROSE.len() as u16 * 4);
    Art {
        logo,
        logo_size: (logo_w, TEXT_LOGO.len() as u16 * 2),
        rose_at: bloom_times(&rose, rose_size),
        rose,
        rose_shade,
        rose_size,
    }
});

/// How far the wordmark's italic leans: a stroke moves this many pixels
/// right per pixel up, so `x + SLANT × y` is constant along it.
const SLANT: f32 = 0.8;

/// When a pen writing the wordmark reaches each of `pixels`, 0..=1. It sweeps
/// along the italic's slant at a steady ink rate: it takes its time over the
/// letters' dense strokes, glides through thin joins, and lifts for a breath
/// over blank columns (the space between the words). `ink` sets the pace.
fn pen_times(ink: &[(u16, u16)], pixels: &[(u16, u16)]) -> Vec<f32> {
    /// Time for a column the pen only passes through.
    const TRAVEL: f32 = 2.0;
    /// Time for a blank column: the pen lifts and moves on.
    const LIFT: f32 = 12.0;
    let u = |&(x, y): &(u16, u16)| x as f32 + SLANT * y as f32;
    let columns = pixels.iter().map(u).fold(0f32, f32::max) as usize + 1;
    let mut weight = vec![0f32; columns];
    for p in ink {
        weight[u(p) as usize] += 1.0;
    }
    let first = ink.iter().map(u).fold(f32::MAX, f32::min) as usize;
    for (i, w) in weight.iter_mut().enumerate() {
        *w = if i < first {
            0.0
        } else if *w == 0.0 {
            LIFT
        } else {
            *w + TRAVEL
        };
    }
    let mut start = Vec::with_capacity(columns);
    let mut total = 0.0;
    for w in &weight {
        start.push(total);
        total += w;
    }
    pixels
        .iter()
        .map(|p| {
            let u = u(p);
            let i = u as usize;
            ((start[i] + u.fract() * weight[i]) / total).clamp(0.0, 1.0)
        })
        .collect()
}

/// When each rose dot opens, 0..=1: from the heart of the bloom outward, so
/// the petals unfold and the stem comes last.
fn bloom_times(dots: &[(u16, u16)], (w, h): (u16, u16)) -> Vec<f32> {
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.32);
    let dist = |&(x, y): &(u16, u16)| ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
    let far = dots.iter().map(dist).fold(1f32, f32::max);
    dots.iter().map(|d| dist(d) / far).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BannerConfig {
    pub show_rose: bool,
    pub show_text_logo: bool,
    pub palette: Palette,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Pink,
    Cyan,
    Yellow,
    Green,
}

impl Palette {
    /// (rose, logo ink, logo shadow)
    fn colors(self) -> (Rgba, Rgba, Rgba) {
        let c = |r: u32, g: u32, b: u32| rgb((r << 16) | (g << 8) | b);
        match self {
            Self::Pink => (c(255, 118, 195), c(255, 118, 195), c(120, 45, 80)),
            Self::Cyan => (c(95, 210, 255), c(105, 220, 255), c(25, 80, 100)),
            Self::Yellow => (c(255, 210, 95), c(255, 215, 105), c(105, 75, 25)),
            Self::Green => (c(110, 220, 145), c(120, 230, 150), c(30, 95, 50)),
        }
    }
}

impl Default for BannerConfig {
    fn default() -> Self {
        Self {
            show_rose: true,
            show_text_logo: true,
            palette: Palette::Pink,
        }
    }
}

/// `<config>/banner.json`, tolerant like the launcher's own read: missing or
/// malformed fields fall back to the defaults.
pub fn read_config(config_home: &Path) -> BannerConfig {
    let default = BannerConfig::default();
    let Ok(raw) = std::fs::read_to_string(config_home.join("banner.json")) else {
        return default;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return default;
    };
    BannerConfig {
        show_rose: value["showRose"].as_bool().unwrap_or(default.show_rose),
        show_text_logo: value["showTextLogo"]
            .as_bool()
            .unwrap_or(default.show_text_logo),
        palette: match value["color"].as_str() {
            Some("cyan") => Palette::Cyan,
            Some("yellow") => Palette::Yellow,
            Some("green") => Palette::Green,
            _ => Palette::Pink,
        },
    }
}

/// The rose's dots (x, y in dot units) and its size, for other effects that
/// draw it (the max-effort surge).
pub fn rose() -> (&'static [(u16, u16)], (u16, u16)) {
    (&ART.rose, ART.rose_size)
}

/// Gentle Shell's pixel rose, `height` px tall, in `color`.
pub fn pixel_rose(height: f32, color: Hsla) -> impl IntoElement {
    let (dots, (width, rows)) = rose();
    let dot = height / rows.max(1) as f32;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let side = px((dot * 0.55).max(1.0));
            for &(x, y) in dots {
                let origin = point(
                    bounds.origin.x + px(x as f32 * dot),
                    bounds.origin.y + px(y as f32 * dot),
                );
                window.paint_quad(fill(Bounds::new(origin, size(side, side)), color));
            }
        },
    )
    .w(px(width as f32 * dot))
    .h(px(height))
}

/// How long the banner takes to bloom and write itself: drive `reveal` from
/// 0 to 1 over this, linearly (the pacing is inside [`banner`]).
pub const WRITE: Duration = Duration::from_millis(2600);

/// Share of `reveal` the rose blooms over.
const BLOOM_UNTIL: f32 = 0.5;
/// Share of `reveal` before the pen touches down.
const PEN_FROM: f32 = 0.14;
/// How long, in pen time, a fresh stroke glows like wet ink.
const WET: f32 = 0.09;
/// Pen time past the last pixel, so the final strokes dry too.
const PEN_TAIL: f32 = WET + 0.03;
/// The rose's float: a slow rise and fall (in wordmark pixels), a sway (in
/// radians), each with its period in seconds, and a glint crossing it.
const BOB: (f32, f32) = (0.9, 5.2);
const SWAY: (f32, f32) = (0.022, 7.3);
const GLINT_PERIOD: f32 = 6.5;
/// The float's redraw rate: slow motion needs no more, and a frame repaints
/// the whole view the banner sits in.
const FLOAT_FRAME: Duration = Duration::from_millis(50);

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `color` lit toward a pale, glowing tint by `by` (0..=1).
fn lighten(color: Hsla, by: f32) -> Hsla {
    Hsla {
        l: color.l + (0.97 - color.l) * by,
        s: color.s * (1.0 - 0.35 * by),
        ..color
    }
}

/// The banner at `scale` px per wordmark pixel. With `reveal` going 0 → 1
/// over [`WRITE`], the rose blooms from its heart outward while a pen writes
/// the wordmark in cursive, each stroke glowing like wet ink before it dries
/// and its shadow settles. The rose floats the whole time: it rises and falls,
/// sways, and a glint crosses its petals (still with reduced motion, and
/// paused while the window is in the background).
pub fn banner(config: BannerConfig, scale: f32, reveal: f32) -> impl IntoElement {
    let art = &*ART;
    let (rose_color, ink, shadow) = config.palette.colors();
    let (rose_base, ink, shadow) = (Hsla::from(rose_color), Hsla::from(ink), Hsla::from(shadow));
    let logo_size = size(
        px(art.logo_size.0 as f32 * scale),
        px(art.logo_size.1 as f32 * scale),
    );
    // Laid out like the combo asset: a terminal cell is one wordmark pixel
    // wide and two tall, and holds 2×4 braille dots, so a dot is half a
    // pixel; the rose then stands 17 cells to the wordmark's 11, centered.
    let dot = scale / 2.0;
    let rose_size = size(
        px(art.rose_size.0 as f32 * dot),
        px(art.rose_size.1 as f32 * dot),
    );
    let reveal = reveal.clamp(0.0, 1.0);

    let rose = canvas(
        |_, window, cx| {
            if !cx.reduce_motion() && window.is_window_active() {
                ui::request_frame_in(FLOAT_FRAME, window, cx);
            }
        },
        move |bounds, _, window, cx| {
            use std::f32::consts::TAU;
            let still = cx.reduce_motion();
            let t = if still { 0.0 } else { ui::live_clock() };
            let bob = if still {
                0.0
            } else {
                (t * TAU / BOB.1).sin() * BOB.0 * scale
            };
            let sway = if still {
                0.0
            } else {
                (t * TAU / SWAY.1 + 0.6).sin() * SWAY.0
            };
            let (sin, cos) = sway.sin_cos();
            // The glint sweeps the diagonal from off one corner to past the other.
            let glint = (t / GLINT_PERIOD).fract() * 1.8 - 0.4;
            let (w, h) = (ART.rose_size.0 as f32, ART.rose_size.1 as f32);
            // It sways about the bloom, like a flower held by its stem.
            let (pivot_x, pivot_y) = (w * 0.5, h * 0.32);
            let bloom = reveal / BLOOM_UNTIL;
            let side = px((dot * 0.55).max(1.0));
            for (i, &(x, y)) in ART.rose.iter().enumerate() {
                let open = smoothstep((bloom - ART.rose_at[i] * 0.75) / 0.25);
                if open <= 0.0 {
                    continue;
                }
                // Petals open outward as they appear.
                let unfold = 0.82 + 0.18 * open;
                let (dx, dy) = ((x as f32 - pivot_x) * unfold, (y as f32 - pivot_y) * unfold);
                let (rx, ry) = (pivot_x + dx * cos - dy * sin, pivot_y + dx * sin + dy * cos);
                let along = (x as f32 / w + y as f32 / h) * 0.5;
                let shine = if still {
                    0.0
                } else {
                    (1.0 - ((along - glint) / 0.12).abs()).max(0.0).powi(2) * 0.35
                };
                let base = Hsla {
                    l: (rose_base.l * ART.rose_shade[i]).min(0.95),
                    a: open,
                    ..rose_base
                };
                let origin = point(
                    bounds.origin.x + px(rx * dot),
                    bounds.origin.y + px(ry * dot + bob),
                );
                window.paint_quad(fill(
                    Bounds::new(origin, size(side, side)),
                    lighten(base, shine),
                ));
            }
        },
    )
    .w(rose_size.width)
    .h(rose_size.height);

    let logo = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // The pen eases in and out, like a hand starting and finishing a
            // signature; its pace along the word is in each pixel's `at`.
            let pen = ((reveal - PEN_FROM) / (1.0 - PEN_FROM)).clamp(0.0, 1.0);
            let pen = (0.5 - 0.5 * (pen * std::f32::consts::PI).cos()) * (1.0 + PEN_TAIL);
            let side = size(px(scale), px(scale));
            for p in &ART.logo {
                let since = pen - p.at;
                let color = if p.shadow {
                    // The shadow settles in once the ink above it has dried a little.
                    Hsla {
                        a: smoothstep((since - 0.04) / 0.07),
                        ..shadow
                    }
                } else {
                    let wet = 1.0 - (since / WET).clamp(0.0, 1.0);
                    Hsla {
                        a: smoothstep(since / 0.02),
                        ..lighten(ink, wet * 0.75)
                    }
                };
                if color.a <= 0.0 {
                    continue;
                }
                let origin = point(
                    bounds.origin.x + px(p.x as f32 * scale),
                    bounds.origin.y + px(p.y as f32 * scale),
                );
                window.paint_quad(fill(Bounds::new(origin, side), color));
            }
        },
    )
    .w(logo_size.width)
    .h(logo_size.height);

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(scale * GAP))
        .when(config.show_rose, |d| d.child(rose))
        .when(config.show_text_logo, |d| d.child(logo))
}

use gpui_kit::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::{
        ART, BannerConfig, Palette, ROSE, ROSE_SHADE, bloom_times, pen_times, read_config,
    };

    #[test]
    fn pen_writes_left_to_right_and_lifts_over_blank_columns() {
        // Two strokes on one row, a blank stretch between them.
        let ink = [(0, 0), (1, 0), (2, 0), (12, 0), (13, 0)];
        let at = pen_times(&ink, &ink);
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?}");
        assert!(at.iter().all(|t| (0.0..=1.0).contains(t)));
        // Crossing the blank takes longer than writing a pixel.
        assert!(at[3] - at[2] > 3.0 * (at[2] - at[1]), "{at:?}");

        // The real wordmark: "Gentle" is done before "Shell" starts.
        let gentle = ART.logo.iter().filter(|p| !p.shadow && p.x < 50);
        let shell = ART.logo.iter().filter(|p| !p.shadow && p.x > 70);
        let gentle_end = gentle.map(|p| p.at).fold(0f32, f32::max);
        let shell_start = shell.map(|p| p.at).fold(1f32, f32::min);
        assert!(gentle_end < shell_start, "{gentle_end} vs {shell_start}");
    }

    #[test]
    fn rose_blooms_from_its_heart_outward() {
        let dots = [(10, 6), (10, 18), (0, 0)];
        let at = bloom_times(&dots, (20, 20));
        assert!(at[0] < at[1] && at[1] < at[2], "{at:?}");
    }

    #[test]
    fn art_decodes_to_pixels() {
        let art = &*ART;
        assert_eq!(art.logo_size.1, 22);
        assert!(art.logo_size.0 > 100);
        assert!(art.logo.iter().any(|p| p.shadow) && art.logo.iter().any(|p| !p.shadow));
        // `⣿` sets all eight dots.
        assert!(art.rose.len() > 500);
        assert_eq!(art.rose_size.1, ROSE.len() as u16 * 4);
    }

    #[test]
    fn every_rose_cell_has_a_shade() {
        assert_eq!(ROSE.len(), ROSE_SHADE.len());
        for (line, shades) in ROSE.iter().zip(ROSE_SHADE) {
            let mut shades = shades.chars().chain(std::iter::repeat(' '));
            for ch in line.chars() {
                let shade = shades.next().unwrap();
                let dots = ch != ' ' && ch != '\u{2800}';
                assert_eq!(dots, shade.is_ascii_digit(), "{line:?} at {ch:?}");
            }
        }
    }

    #[test]
    fn config_falls_back_per_field() {
        let dir = std::env::temp_dir().join(format!("gs-banner-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_config(&dir), BannerConfig::default());
        std::fs::write(
            dir.join("banner.json"),
            r#"{"showRose":false,"color":"cyan"}"#,
        )
        .unwrap();
        assert_eq!(
            read_config(&dir),
            BannerConfig {
                show_rose: false,
                show_text_logo: true,
                palette: Palette::Cyan
            }
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
