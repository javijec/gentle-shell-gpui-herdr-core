//! Effects on the theme's picture itself, after Wallpaper Engine's Water
//! Flow and Foliage Sway: a region of the picture keeps flowing (water,
//! clouds) or ripples in place (leaves), so the painting's own pixels move.
//!
//! GPUI has no custom shaders, so this runs on the CPU in the scene thread:
//! each effect's region is prepared once (its pixels, soft-edged mask and
//! per-pixel phase), and each frame renders it into a small patch the window
//! paints over the picture where the camera puts it. A heavy effect slows
//! the scene's frame rate, like a heavy scene, instead of costing more.

use std::sync::Arc;

use super::model::{Effect, EffectKind};

/// The most pixels one effect's patch renders per frame; a larger region is
/// rendered at a lower resolution (and drawn scaled up).
const PATCH_BUDGET: f32 = 120_000.;

/// The most pixels all effects render per frame together, so a theme with
/// many regions costs about what one with four does (every patch is also
/// uploaded to the GPU each frame).
const TOTAL_BUDGET: f32 = 480_000.;

/// The theme's picture, decoded once.
pub struct Picture {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Picture {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let image = image::load_from_memory(bytes).ok()?.to_rgba8();
        Some(Self {
            width: image.width() as usize,
            height: image.height() as usize,
            rgba: image.into_raw(),
        })
    }

    /// Bilinear sample at image pixel coordinates, clamped to the picture.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = x.clamp(0., (self.width - 1) as f32);
        let y = y.clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let (top, bottom) = (y0 * self.width, y1 * self.width);
        let corners = [
            ((top + x0) * 4, (1. - fx) * (1. - fy)),
            ((top + x1) * 4, fx * (1. - fy)),
            ((bottom + x0) * 4, (1. - fx) * fy),
            ((bottom + x1) * 4, fx * fy),
        ];
        let mut out = [0.; 4];
        for (at, weight) in corners {
            let pixel = &self.rgba[at..at + 4];
            for (value, channel) in out.iter_mut().zip(pixel) {
                *value += f32::from(*channel) * weight;
            }
        }
        out
    }
}

/// One rendered effect: `width` × `height` BGRA pixels with straight alpha
/// (the region's soft mask), covering `rect` of the picture (`[u0, v0, u1,
/// v1]`, 0–1).
#[derive(Clone)]
pub struct Patch {
    pub rect: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub bgra: Arc<Vec<u8>>,
}

/// An effect ready to render: where its patch sits in the picture, at what
/// resolution, and what doesn't change from frame to frame.
struct Prepared {
    effect: Effect,
    /// The patch's top-left corner, in picture pixels.
    left: f32,
    top: f32,
    /// Picture pixels per patch pixel.
    step: f32,
    width: usize,
    height: usize,
    /// Per patch pixel: the mask (0–1) and the phase (0–1).
    mask: Vec<f32>,
    phase: Vec<f32>,
    /// A light's region fully lit, per patch pixel; empty for the others.
    lit: Vec<[u8; 3]>,
}

/// The picture's effects, prepared for the scale they're shown at.
pub struct Effects {
    picture: Picture,
    effects: Vec<Effect>,
    prepared: Vec<Prepared>,
    /// The (rounded) scale `prepared` was made for: window pixels per
    /// picture pixel. Nothing finer than that is rendered.
    shown: f32,
}

impl Effects {
    pub fn new(picture: Picture, effects: &[Effect]) -> Self {
        Self {
            picture,
            effects: effects.to_vec(),
            prepared: Vec::new(),
            shown: 0.,
        }
    }

    /// Every effect at scene time `t`, for a picture shown at `shown` window
    /// pixels per picture pixel (the camera's scale). Changing the scale
    /// noticeably (a resized window) prepares the effects again.
    pub fn render(&mut self, t: f64, shown: f32) -> Vec<Patch> {
        self.prepare(shown);
        self.prepared
            .iter()
            .map(|p| render(&self.picture, p, t))
            .collect()
    }

    /// The `index`th rendered effect at scene time `t`, at full resolution,
    /// over the picture under its region: opaque RGBA, with its width and
    /// height. For previews.
    pub fn moment(&mut self, index: usize, t: f64) -> Option<(u32, u32, Vec<u8>)> {
        self.prepare(1.);
        let prepared = self.prepared.get(index)?;
        let patch = render(&self.picture, prepared, t);
        let mut rgba = Vec::with_capacity(patch.bgra.len());
        for (i, over) in patch.bgra.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = ((i % prepared.width) as f32, (i / prepared.width) as f32);
            let under = self.picture.sample(
                prepared.left + (x + 0.5) * prepared.step - 0.5,
                prepared.top + (y + 0.5) * prepared.step - 0.5,
            );
            let alpha = f32::from(over[3]) / 255.;
            for (channel, under) in [over[2], over[1], over[0]].into_iter().zip(under) {
                rgba.push((f32::from(channel) * alpha + under * (1. - alpha)).round() as u8);
            }
            rgba.push(255);
        }
        Some((patch.width, patch.height, rgba))
    }

    /// Prepare every effect for a picture shown at `shown` window pixels per
    /// picture pixel, unless it already is.
    fn prepare(&mut self, shown: f32) {
        // Quarter steps of a power of two: a camera's slow zoom doesn't
        // re-prepare, a resized window does.
        let shown = 2f32
            .powf((shown.max(0.01).log2() * 4.).round() / 4.)
            .min(1.);
        if shown != self.shown {
            self.shown = shown;
            let budget = PATCH_BUDGET.min(TOTAL_BUDGET / self.effects.len().max(1) as f32);
            self.prepared = self
                .effects
                .iter()
                .filter_map(|effect| prepare(&self.picture, effect, 1. / shown, budget))
                .collect();
        }
    }
}

/// `effect` ready to render in at most `budget` pixels, never finer than
/// `finest` picture pixels per patch pixel.
fn prepare(picture: &Picture, effect: &Effect, finest: f32, budget: f32) -> Option<Prepared> {
    let (w, h) = (picture.width as f32, picture.height as f32);
    let points: Vec<(f32, f32)> = effect.region.iter().map(|(u, v)| (u * w, v * h)).collect();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0f32, 0f32);
    for (x, y) in &points {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    let (bw, bh) = (x1 - x0, y1 - y0);
    if bw < 2. || bh < 2. {
        return None;
    }
    let step = (bw * bh / budget).sqrt().max(finest).max(1.);
    let (width, height) = ((bw / step).ceil() as usize, (bh / step).ceil() as usize);
    let feather = (effect.feather * w).max(0.5);
    let mut mask = Vec::with_capacity(width * height);
    let mut phase = Vec::with_capacity(width * height);
    for py in 0..height {
        for px in 0..width {
            let (x, y) = (x0 + (px as f32 + 0.5) * step, y0 + (py as f32 + 0.5) * step);
            let inside = contains(&points, x, y);
            mask.push(if inside {
                smoothstep(edge_distance(&points, x, y) / feather)
            } else {
                0.
            });
            // Low-frequency noise across the region, in region units, so
            // neighbors move together and distant parts don't move in unison.
            let (u, v) = ((x - x0) / bw, (y - y0) / bh);
            phase.push(match effect.kind {
                EffectKind::Flow { .. } => value_noise(u * 3.1, v * 3.1),
                EffectKind::Sway { scale, .. } => value_noise(u * scale, v * scale * bh / bw),
                EffectKind::Light { .. } => value_noise(u * 2.3, v * 2.3 * bh / bw),
            });
        }
    }
    let lit = match effect.kind {
        EffectKind::Light { .. } => {
            let grid = Grid {
                left: x0,
                top: y0,
                step,
                width,
                height,
            };
            light(picture, &grid, effect)
        }
        _ => Vec::new(),
    };
    Some(Prepared {
        effect: effect.clone(),
        left: x0,
        top: y0,
        step,
        width,
        height,
        mask,
        phase,
        lit,
    })
}

/// Where a patch samples the picture: its top-left corner and picture
/// pixels per patch pixel, and its size in patch pixels.
struct Grid {
    left: f32,
    top: f32,
    step: f32,
    width: usize,
    height: usize,
}

/// A light's region fully lit: each pixel brightened by how far it's past
/// the threshold, plus those bright parts blurred out as a bloom, in the
/// light's color or their own.
fn light(picture: &Picture, grid: &Grid, effect: &Effect) -> Vec<[u8; 3]> {
    let EffectKind::Light {
        color,
        threshold,
        warm,
        glow,
        ..
    } = effect.kind
    else {
        return Vec::new();
    };
    let strength = effect.strength;
    let radius = (glow * picture.width as f32 / grid.step).round() as usize;
    let mut base = Vec::with_capacity(grid.width * grid.height);
    let mut luma = Vec::with_capacity(grid.width * grid.height);
    for py in 0..grid.height {
        for px in 0..grid.width {
            let c = picture.sample(
                grid.left + (px as f32 + 0.5) * grid.step,
                grid.top + (py as f32 + 0.5) * grid.step,
            );
            luma.push((0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]) / 255.);
            base.push([c[0], c[1], c[2], 0.]);
        }
    }
    // A light stands out from what's around it: a flame against its dark
    // window, not a sunlit wall or a bush of autumn leaves.
    let mut around = luma.clone();
    box_blur(&mut around, grid.width, grid.height, (radius / 2).max(2));
    // How much each pixel looks like a light: bright, brighter than what's
    // around it, and (for `warm`) redder than blue, as lamplight and fire
    // are and pale stone and sky aren't.
    let mut score: Vec<f32> = base
        .iter()
        .zip(&luma)
        .zip(&around)
        .map(|((b, luma), around)| {
            let mut k = smoothstep((luma - threshold) / 0.25) * smoothstep((luma - around) / 0.2);
            if warm {
                k *= smoothstep(((b[0] - b[2]) / 255. - 0.03) / 0.25);
            }
            k
        })
        .collect();
    // The region is drawn around its lights, so its most light-like pixels
    // are the lights, however dim the painting draws them.
    let top = score.iter().copied().fold(0f32, f32::max);
    if top <= 1e-3 {
        return base
            .iter()
            .map(|b| [b[0], b[1], b[2]].map(|v| v as u8))
            .collect();
    }
    for k in &mut score {
        *k = smoothstep((*k / top - 0.7) / 0.3);
    }
    // The sources' own color, weighted by how much each is a source.
    let (mut own, mut weight) = ([0f32; 3], 0f32);
    for (b, k) in base.iter_mut().zip(&score) {
        b[3] = *k;
        own = std::array::from_fn(|i| own[i] + b[i] * k);
        weight += k;
    }
    let mut halo = score;
    let tint = color.map_or_else(
        || own.map(|v| v / weight),
        |c| [(c >> 16) & 255, (c >> 8) & 255, c & 255].map(|v| v as f32),
    );
    // Three box blurs come close to a Gaussian. Each light is measured
    // against a lamp a third of `glow` across, so a candle flame glows
    // nearly as far as a lit window and one big source doesn't drown the
    // small ones.
    for _ in 0..3 {
        box_blur(&mut halo, grid.width, grid.height, radius);
    }
    let full = reference_peak(radius);
    base.iter()
        .zip(&halo)
        .map(|(b, h)| {
            // The running sums leave tiny residues, below zero too.
            let reach = (h / full - 0.01).clamp(0., 1.);
            let glow = (strength * 0.6 * reach.powf(0.5)).min(1.);
            let lift = 1. + 0.5 * strength * b[3];
            // Screen: light adds up toward white without clipping.
            std::array::from_fn(|c| {
                let lit = (b[c] * lift).min(255.);
                (255. - (255. - lit) * (1. - tint[c] / 255. * glow)) as u8
            })
        })
        .collect()
}

/// The center of a lamp a third of `radius` across, after the three blurs.
fn reference_peak(radius: usize) -> f32 {
    let size = 6 * radius + 1;
    let lamp = (radius / 6).max(1) as f32;
    let middle = (size / 2) as f32;
    let mut disc: Vec<f32> = (0..size * size)
        .map(|i| {
            let (x, y) = ((i % size) as f32 - middle, (i / size) as f32 - middle);
            if x.hypot(y) <= lamp { 1. } else { 0. }
        })
        .collect();
    for _ in 0..3 {
        box_blur(&mut disc, size, size, radius);
    }
    disc[size * size / 2].max(1e-6)
}

/// Blur `values` (`width` × `height`) in place with a box `radius` pixels
/// out each way, rows then columns, clamped at the edges.
fn box_blur(values: &mut [f32], width: usize, height: usize, radius: usize) {
    if radius == 0 {
        return;
    }
    let size = (2 * radius + 1) as f32;
    let mut line = Vec::new();
    let mut pass =
        |values: &mut [f32], count: usize, length: usize, at: &dyn Fn(usize, usize) -> usize| {
            for n in 0..count {
                line.clear();
                line.extend((0..length).map(|k| values[at(n, k)]));
                let get = |k: isize| line[k.clamp(0, length as isize - 1) as usize];
                let reach = radius as isize;
                let mut sum: f32 = (-reach..=reach).map(get).sum();
                for k in 0..length {
                    values[at(n, k)] = sum / size;
                    let k = k as isize;
                    sum += get(k + reach + 1) - get(k - reach);
                }
            }
        };
    pass(values, height, width, &|row, k| row * width + k);
    pass(values, width, height, &|column, k| k * width + column);
}

fn render(picture: &Picture, p: &Prepared, t: f64) -> Patch {
    let (pw, ph) = (picture.width as f32, picture.height as f32);
    let mut bgra = vec![0u8; p.width * p.height * 4];
    let cycles = t * f64::from(p.effect.speed);
    for py in 0..p.height {
        for px in 0..p.width {
            let i = py * p.width + px;
            let mask = p.mask[i];
            if mask <= 0. {
                continue;
            }
            let (x, y) = (
                p.left + (px as f32 + 0.5) * p.step,
                p.top + (py as f32 + 0.5) * p.step,
            );
            let color = match p.effect.kind {
                // Flow map: two copies sliding along the direction half a
                // cycle apart, each faded out as it jumps back, so the loop
                // never shows a seam.
                EffectKind::Flow { direction } => {
                    let travel = p.effect.strength * ph;
                    let time = cycles + f64::from(p.phase[i]) * 0.8;
                    let a = time.fract() as f32;
                    let b = (time + 0.5).fract() as f32;
                    let at = |k: f32| {
                        let d = (k - 0.5) * travel;
                        picture.sample(x - direction.0 * d, y - direction.1 * d)
                    };
                    let weight = 1. - (2. * a - 1.).abs();
                    mix(at(b), at(a), weight)
                }
                // Each spot swings on its own phase; the swing grows and
                // fades with a slower wave, like gusts.
                EffectKind::Sway { ratio, .. } => {
                    let reach = p.effect.strength * pw;
                    let angle = std::f64::consts::TAU * (cycles + f64::from(p.phase[i]));
                    let gust = 0.65 + 0.35 * (angle * 0.23 + 1.3).sin() as f32;
                    let dx = reach * gust * angle.sin() as f32;
                    let dy = reach * gust * ratio * (angle * 1.3).cos() as f32;
                    picture.sample(x + dx, y + dy)
                }
                // The lit region over the picture, its alpha dipping on a
                // slow wave with a quicker one on top, each spot on its phase.
                EffectKind::Light { flicker, .. } => {
                    let phase = p.phase[i];
                    let time = (cycles + f64::from(phase) * 9.7) as f32;
                    let wave =
                        0.7 * value_noise(time, phase * 31.) + 0.3 * value_noise(time * 3.7, 7.);
                    let [r, g, b] = p.lit[i].map(f32::from);
                    [r, g, b, 255. * (1. - flicker * wave)]
                }
            };
            let o = i * 4;
            bgra[o] = color[2] as u8;
            bgra[o + 1] = color[1] as u8;
            bgra[o + 2] = color[0] as u8;
            bgra[o + 3] = (color[3] * mask) as u8;
        }
    }
    Patch {
        rect: [
            p.left / pw,
            p.top / ph,
            (p.left + p.width as f32 * p.step) / pw,
            (p.top + p.height as f32 * p.step) / ph,
        ],
        width: p.width as u32,
        height: p.height as u32,
        bgra: Arc::new(bgra),
    }
}

fn mix(a: [f32; 4], b: [f32; 4], k: f32) -> [f32; 4] {
    std::array::from_fn(|c| a[c] + (b[c] - a[c]) * k)
}

fn smoothstep(k: f32) -> f32 {
    let k = k.clamp(0., 1.);
    k * k * (3. - 2. * k)
}

/// Whether (x, y) is inside the polygon (even-odd).
fn contains(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut j = points.len() - 1;
    for (i, &(xi, yi)) in points.iter().enumerate() {
        let (xj, yj) = points[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The distance from (x, y) to the polygon's nearest edge.
fn edge_distance(points: &[(f32, f32)], x: f32, y: f32) -> f32 {
    let mut best = f32::MAX;
    for i in 0..points.len() {
        let (ax, ay) = points[i];
        let (bx, by) = points[(i + 1) % points.len()];
        let (dx, dy) = (bx - ax, by - ay);
        let length = dx * dx + dy * dy;
        let k = if length > 0. {
            (((x - ax) * dx + (y - ay) * dy) / length).clamp(0., 1.)
        } else {
            0.
        };
        best = best.min((x - ax - k * dx).hypot(y - ay - k * dy));
    }
    best
}

/// Smooth value noise in 0–1.
fn value_noise(x: f32, y: f32) -> f32 {
    let hash = |x: i32, y: i32| {
        let h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
        let h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        (h ^ (h >> 16)) as f32 / u32::MAX as f32
    };
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (smoothstep(x - x.floor()), smoothstep(y - y.floor()));
    let top = hash(xi, yi) + (hash(xi + 1, yi) - hash(xi, yi)) * fx;
    let bottom = hash(xi, yi + 1) + (hash(xi + 1, yi + 1) - hash(xi, yi + 1)) * fx;
    top + (bottom - top) * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture of horizontal stripes 8 rows wide, red and black: a ramp
    /// would hide a flow (its two sliding copies average back to it).
    fn stripes(width: usize, height: usize) -> Picture {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for _ in 0..width {
                rgba.extend([if y / 8 % 2 == 0 { 250 } else { 0 }, 0, 0, 255]);
            }
        }
        Picture {
            width,
            height,
            rgba,
        }
    }

    fn square(kind: EffectKind, strength: f32) -> Effect {
        Effect {
            kind,
            region: vec![(0.25, 0.25), (0.75, 0.25), (0.75, 0.75), (0.25, 0.75)],
            feather: 0.,
            speed: 1.,
            strength,
        }
    }

    #[test]
    fn flow_moves_the_regions_pixels_and_leaves_the_rest_clear() {
        let mut effects = Effects::new(
            stripes(200, 200),
            &[square(
                EffectKind::Flow {
                    direction: (0., 1.),
                },
                0.2,
            )],
        );
        let mut at = |t: f64| effects.render(t, 1.).remove(0);
        let patch = at(0.);
        assert_eq!(patch.rect, [0.25, 0.25, 0.75, 0.75]);
        // A pixel well inside shows content from elsewhere in its column,
        // and it changes over time: the stripes slide.
        let middle = |p: &Patch| {
            let i = ((p.height / 2) * p.width + p.width / 2) as usize * 4;
            (p.bgra[i + 2], p.bgra[i + 3])
        };
        let (red0, alpha) = middle(&patch);
        assert_eq!(alpha, 255);
        let red1 = middle(&at(0.3)).0;
        assert_ne!(red0, red1, "the water moves");
        // Corners of the patch's bounding box are inside the square here,
        // but a triangle would leave them clear: the mask follows the region.
        let triangle = Effect {
            region: vec![(0.25, 0.25), (0.75, 0.25), (0.25, 0.75)],
            ..square(
                EffectKind::Flow {
                    direction: (0., 1.),
                },
                0.2,
            )
        };
        let patch = Effects::new(stripes(200, 200), &[triangle])
            .render(0., 1.)
            .remove(0);
        let last = (patch.width * patch.height - 1) as usize * 4;
        assert_eq!(patch.bgra[last + 3], 0, "outside the region stays clear");
    }

    #[test]
    fn a_big_region_is_rendered_within_the_budget() {
        let mut effects = Effects::new(
            stripes(2000, 1500),
            &[Effect {
                region: vec![(0., 0.), (1., 0.), (1., 1.), (0., 1.)],
                ..square(
                    EffectKind::Sway {
                        scale: 12.,
                        ratio: 0.3,
                    },
                    0.003,
                )
            }],
        );
        let patch = effects.render(1., 1.).remove(0);
        assert!((patch.width * patch.height) as f32 <= PATCH_BUDGET * 1.01);
        assert!(
            patch
                .rect
                .iter()
                .zip([0., 0., 1., 1.])
                .all(|(a, b)| (a - b).abs() < 0.01)
        );
        // Shown at half size, it renders no finer than the window shows it.
        let small = Effects::new(
            stripes(400, 300),
            &[Effect {
                region: vec![(0., 0.), (1., 0.), (1., 1.), (0., 1.)],
                ..square(
                    EffectKind::Sway {
                        scale: 12.,
                        ratio: 0.3,
                    },
                    0.003,
                )
            }],
        )
        .render(1., 0.5)
        .remove(0);
        assert_eq!((small.width, small.height), (200, 150));
    }

    #[test]
    fn a_light_glows_onto_its_surroundings_and_flickers() {
        // A dark wall with one small warm lamp in the middle, and a pale
        // (not warm) patch that must not light up.
        let (w, h) = (200, 200);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let lamp = x.abs_diff(100) <= 2 && y.abs_diff(100) <= 2;
                let pale = x < 70 && y < 70;
                rgba.extend(match (lamp, pale) {
                    (true, _) => [255, 190, 90, 255],
                    (_, true) => [200, 200, 205, 255],
                    _ => [40, 35, 30, 255],
                });
            }
        }
        let light = Effect {
            region: vec![(0.2, 0.2), (0.8, 0.2), (0.8, 0.8), (0.2, 0.8)],
            ..square(
                EffectKind::Light {
                    color: None,
                    threshold: 0.45,
                    warm: true,
                    glow: 0.05,
                    flicker: 0.6,
                },
                1.,
            )
        };
        let mut effects = Effects::new(
            Picture {
                width: w,
                height: h,
                rgba,
            },
            &[light],
        );
        let (pw, _, lit) = effects.moment(0, 0.).unwrap();
        let red = |x: usize, y: usize| {
            // Patch pixels from the region's corner at (40, 40).
            lit[((y - 40) * pw as usize + (x - 40)) * 4]
        };
        assert!(
            red(108, 100) > 70,
            "beside the lamp is lit: {}",
            red(108, 100)
        );
        assert!(
            red(150, 150) < 50,
            "far from it stays dark: {}",
            red(150, 150)
        );
        assert!(
            red(50, 50) < 215,
            "the pale wall isn't a light: {}",
            red(50, 50)
        );
        let alphas: Vec<u8> = (0..20)
            .map(|k| {
                effects.render(f64::from(k) * 0.37, 1.).remove(0).bgra
                    [(60 * pw as usize + 60) * 4 + 3]
            })
            .collect();
        assert!(
            alphas.iter().max().unwrap() - alphas.iter().min().unwrap() > 20,
            "it flickers: {alphas:?}"
        );
    }
}
