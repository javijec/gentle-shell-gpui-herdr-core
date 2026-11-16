//! A theme's scene, run in a sandboxed QuickJS on its own thread.
//!
//! The scene is JavaScript (see `kit.js` for what it can call). QuickJS here
//! has no file, network, timer or module access: a scene can only compute
//! and leave drawing commands, which this thread hands to the window as
//! [`Frame`]s. Each frame is cut off after [`FRAME_BUDGET`] and the runtime is
//! capped at [`MEMORY_LIMIT`], so a broken or hostile scene can't hang or
//! exhaust the app; after a few failures the scene stops.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rquickjs::{Context, Function, Object, Runtime, TypedArray};

const KIT: &str = include_str!("kit.js");

/// Numbers per drawing command in a frame (`op` and up to 9 arguments).
pub const STRIDE: usize = 10;
/// How long one frame of the scene may compute.
const FRAME_BUDGET: Duration = Duration::from_millis(250);
/// Memory for the whole JS runtime.
const MEMORY_LIMIT: usize = 64 << 20;
/// The fastest a scene is computed while it moves: 24 fps, smooth for
/// ambient motion at a fifth less work than 30.
const FASTEST: Duration = Duration::from_millis(42);
/// The slowest a heavy scene drops to: 8 fps.
const SLOWEST: Duration = Duration::from_millis(125);
/// The share of one core a scene's script may take; a heavier scene is
/// computed less often instead of costing more.
const CPU_SHARE: f64 = 0.08;
/// Failed frames in a row before the scene is given up.
const MAX_FAILURES: u32 = 3;

/// One computed frame: `STRIDE` numbers per command, the x, y pairs its paths
/// point into (`kit.js` documents both), where the theme's image sits in it
/// (`[x, y, w, h]`, moved by its camera) and its effects' patches, for a
/// window of `size`.
pub struct Frame {
    pub commands: Vec<f64>,
    pub points: Vec<f64>,
    pub image: Option<[f32; 4]>,
    pub patches: Vec<super::effects::Patch>,
    pub size: (f32, f32),
}

/// What the window tells the scene and the scene leaves for the window.
#[derive(Default)]
struct Shared {
    frame: Mutex<Option<Arc<Frame>>>,
    /// The window's size in logical pixels, as `f32` bits.
    width: AtomicU64,
    height: AtomicU64,
    /// Time moves (window focused, motion allowed, speed above 0).
    moving: AtomicBool,
    /// The scene's thread should end.
    stop: AtomicBool,
    /// Milliseconds between frames, as the scene's cost allows.
    interval: AtomicU64,
    /// The least milliseconds between frames the person allows (Calm).
    floor: AtomicU64,
}

/// How long to wait between frames of a scene that takes `cost` each, at
/// least `floor`.
pub(super) fn pace(cost: Duration, floor: Duration) -> Duration {
    cost.div_f64(CPU_SHARE).clamp(FASTEST, SLOWEST).max(floor)
}

/// A running scene. Dropping it ends its thread.
pub struct Scene {
    shared: Arc<Shared>,
}

/// What a scene needs besides its code.
pub struct Setup {
    /// The theme's name, for reporting a failure.
    pub theme: String,
    pub code: Arc<str>,
    /// The ASCII cell size in pixels.
    pub cell: f32,
    /// Seconds of scene time per real second.
    pub speed: f32,
    /// The theme's colors, as a JSON object (`g.theme` in the scene).
    pub colors: String,
    /// The theme's image size and camera, so each frame places the image
    /// and the scene can draw on it (`g.image`, `g.fromImage`).
    pub image: Option<((f32, f32), Option<super::model::Camera>)>,
    /// The image's bytes and its effects, decoded and prepared on the
    /// scene's thread.
    pub effects: Option<(Arc<gpui_kit::Image>, Vec<super::model::Effect>)>,
}

impl Scene {
    pub fn start(setup: Setup) -> Self {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let theme = setup.theme.clone();
        let spawned = std::thread::Builder::new()
            .name("theme-scene".into())
            .spawn(move || {
                let theme = setup.theme.clone();
                if let Err(err) = run(&worker, setup) {
                    super::report(&theme, Some(err));
                }
            });
        if let Err(err) = spawned {
            super::report(&theme, Some(err.to_string()));
        }
        Self { shared }
    }

    /// The latest frame, if any.
    pub fn frame(&self) -> Option<Arc<Frame>> {
        self.shared.frame.lock().unwrap().clone()
    }

    pub fn set_size(&self, width: f32, height: f32) {
        self.shared
            .width
            .store(f64::from(width).to_bits(), Ordering::Relaxed);
        self.shared
            .height
            .store(f64::from(height).to_bits(), Ordering::Relaxed);
    }

    pub fn set_moving(&self, moving: bool) {
        self.shared.moving.store(moving, Ordering::Relaxed);
    }

    /// Never compute frames more often than every `floor`.
    pub fn set_floor(&self, floor: Duration) {
        self.shared
            .floor
            .store(floor.as_millis() as u64, Ordering::Relaxed);
    }

    /// How often the window should show a new frame.
    pub fn interval(&self) -> Duration {
        match self.shared.interval.load(Ordering::Relaxed) {
            0 => FASTEST,
            ms => Duration::from_millis(ms),
        }
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

fn js_error(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    if let rquickjs::Error::Exception = err {
        let caught = ctx.catch();
        if let Some(exception) = caught.as_exception() {
            return exception.to_string();
        }
        if let Some(text) = caught.as_string().and_then(|s| s.to_string().ok()) {
            return text;
        }
        return "the scene threw".to_string();
    }
    err.to_string()
}

/// A scene loaded in its own QuickJS runtime, ready to draw frames.
pub struct Engine {
    // `context` before `runtime`: a context must go before its runtime.
    context: Context,
    _runtime: Runtime,
    epoch: Instant,
    /// Milliseconds since `epoch` after which running JS is cut off.
    deadline: Arc<AtomicU64>,
    cell: f32,
}

impl Engine {
    pub fn new(code: &str, colors: &str, cell: f32) -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(MEMORY_LIMIT);
        runtime.set_max_stack_size(1 << 20);
        let epoch = Instant::now();
        let deadline = Arc::new(AtomicU64::new(u64::MAX));
        let limit = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            epoch.elapsed().as_millis() as u64 > limit.load(Ordering::Relaxed)
        })));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        let engine = Self {
            context,
            _runtime: runtime,
            epoch,
            deadline,
            cell,
        };
        engine.arm(Duration::from_secs(2));
        engine.context.with(|ctx| -> Result<(), String> {
            ctx.eval::<(), _>(KIT).map_err(|e| js_error(&ctx, e))?;
            // A block, so the scene's own names don't clash with the kit's;
            // opened on the first line so errors name the file's own lines.
            ctx.eval::<(), _>(format!("{{{code}\n}}"))
                .map_err(|e| js_error(&ctx, e))?;
            let api: Object = ctx.globals().get("__api").map_err(|e| e.to_string())?;
            let pick: Function = api.get("pick").map_err(|e| e.to_string())?;
            if !pick.call::<_, bool>(()).map_err(|e| js_error(&ctx, e))? {
                return Err("the scene adds nothing to SCENES".to_string());
            }
            let set_theme: Function = api.get("setTheme").map_err(|e| e.to_string())?;
            let colors = ctx
                .json_parse(colors.to_string())
                .map_err(|e| js_error(&ctx, e))?;
            set_theme
                .call::<_, ()>((colors,))
                .map_err(|e| js_error(&ctx, e))
        })?;
        Ok(engine)
    }

    fn arm(&self, budget: Duration) {
        self.deadline.store(
            (self.epoch.elapsed() + budget).as_millis() as u64,
            Ordering::Relaxed,
        );
    }

    /// The commands and path points for a `size` window at scene time `time`,
    /// with the theme's image at `image` (`[x, y, w, h]`), if it has one.
    pub fn frame(
        &self,
        size: (f32, f32),
        time: f64,
        image: Option<[f32; 4]>,
    ) -> Result<(Vec<f64>, Vec<f64>), String> {
        self.arm(FRAME_BUDGET);
        self.context.with(|ctx| {
            let api: Object = ctx.globals().get("__api").map_err(|e| e.to_string())?;
            let frame: Function = api.get("frame").map_err(|e| e.to_string())?;
            let image = image.map(|rect| rect.to_vec());
            let count = frame
                .call::<_, f64>((size.0, size.1, time, self.cell, image))
                .map_err(|e| js_error(&ctx, e))? as usize;
            let read = |name: &str, len: usize| -> Result<Vec<f64>, String> {
                let get: Function = api.get(name).map_err(|e| e.to_string())?;
                let buffer: TypedArray<f64> = get.call(()).map_err(|e| js_error(&ctx, e))?;
                let bytes = buffer.arraybuffer().map_err(|e| e.to_string())?;
                // SAFETY: no JS runs while `bytes` is borrowed, so nothing can
                // detach or resize the buffer under it; it's copied right away.
                let bytes = unsafe { bytes.as_bytes() }.ok_or("the frame buffer is gone")?;
                let used = (len * 8).min(bytes.len());
                Ok(bytes[..used]
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| f64::from_ne_bytes(*b))
                    .collect())
            };
            let pairs: Function = api.get("pointCount").map_err(|e| e.to_string())?;
            let pairs = pairs.call::<_, f64>(()).map_err(|e| js_error(&ctx, e))? as usize;
            Ok((read("out", count * STRIDE)?, read("points", pairs * 2)?))
        })
    }
}

fn run(shared: &Shared, setup: Setup) -> Result<(), String> {
    let engine = Engine::new(&setup.code, &setup.colors, setup.cell)?;
    let mut effects = setup.effects.as_ref().and_then(|(image, effects)| {
        let picture = super::effects::Picture::decode(&image.bytes)?;
        Some(super::effects::Effects::new(picture, effects))
    });
    let mut time = 0f64;
    let mut last = Instant::now();
    let mut failures = 0;
    let mut drawn: Option<(f32, f32)> = None;
    // A running average of what a frame costs.
    let mut cost = Duration::ZERO;
    while !shared.stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        let size = (
            f64::from_bits(shared.width.load(Ordering::Relaxed)) as f32,
            f64::from_bits(shared.height.load(Ordering::Relaxed)) as f32,
        );
        let moving = shared.moving.load(Ordering::Relaxed);
        let elapsed = last.elapsed().min(Duration::from_millis(250));
        last = Instant::now();
        // Held still: draw once per size, then wait.
        if size.0 < 1. || size.1 < 1. || (!moving && drawn == Some(size)) {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if moving {
            time += elapsed.as_secs_f64() * f64::from(setup.speed);
        } else if drawn.is_none() {
            // A still scene shows a moment into it, not its empty start.
            time = 20.;
        }
        let image = setup.image.map(|(natural, camera)| {
            super::model::image_rect(size, natural, camera.as_ref(), time)
        });
        match engine.frame(size, time, image) {
            Ok((commands, points)) => {
                failures = 0;
                if drawn.is_none() {
                    super::report(&setup.theme, None);
                }
                drawn = Some(size);
                // Effects render no finer than the window shows the picture.
                let shown = match (image, setup.image) {
                    (Some(rect), Some(((natural, _), _))) => rect[2] / natural,
                    _ => 1.,
                };
                let patches = effects
                    .as_mut()
                    .map(|e| e.render(time, shown))
                    .unwrap_or_default();
                *shared.frame.lock().unwrap() = Some(Arc::new(Frame {
                    commands,
                    points,
                    image,
                    patches,
                    size,
                }));
            }
            Err(err) => {
                failures += 1;
                if failures >= MAX_FAILURES {
                    return Err(err);
                }
            }
        }
        let spent = started.elapsed();
        cost = if cost.is_zero() {
            spent
        } else {
            (cost * 7 + spent) / 8
        };
        let floor = Duration::from_millis(shared.floor.load(Ordering::Relaxed));
        let interval = pace(cost, floor);
        shared
            .interval
            .store(interval.as_millis() as u64, Ordering::Relaxed);
        if let Some(rest) = interval.checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLORS: &str = r##"{"bg": "#000000", "accent": "#f095c8"}"##;

    #[test]
    fn every_built_in_scene_draws_within_its_budget() {
        for name in super::super::builtin_scene_names() {
            let code = super::super::builtin_scene(name).unwrap();
            let engine = Engine::new(code, COLORS, 22.).unwrap_or_else(|e| panic!("{name}: {e}"));
            for t in [0., 7.5, 31.] {
                let started = Instant::now();
                let (commands, _) = engine
                    .frame((1440., 900.), t, None)
                    .unwrap_or_else(|e| panic!("{name} at {t}: {e}"));
                assert!(commands.len() > STRIDE * 10, "{name} drew almost nothing");
                assert_eq!(commands.len() % STRIDE, 0);
                eprintln!(
                    "{name} t={t}: {} commands in {:?}",
                    commands.len() / STRIDE,
                    started.elapsed()
                );
            }
        }
    }

    #[test]
    fn a_runaway_scene_is_cut_off_and_a_broken_one_says_why() {
        let endless = "SCENES.x = { draw() { for (;;) {} } }";
        let engine = Engine::new(endless, COLORS, 22.).unwrap();
        let started = Instant::now();
        assert!(engine.frame((800., 600.), 0., None).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));

        let broken = Engine::new("SCENES.x = { draw( }", COLORS, 22.)
            .err()
            .unwrap();
        // With where, in the file's own lines.
        assert!(
            broken.contains("missing formal parameter") && broken.contains(":1:"),
            "{broken}"
        );
        let empty = Engine::new("const nothing = 1", COLORS, 22.);
        assert!(empty.is_err_and(|e| e.contains("SCENES")));
    }

    #[test]
    fn scenes_have_no_way_out_of_the_sandbox() {
        let probe = "SCENES.x = { draw(g) { g.text(0, 0, [typeof require, typeof fetch, \
            typeof setTimeout, typeof std, typeof os].join(','), '#fff') } }";
        let engine = Engine::new(probe, COLORS, 22.).unwrap();
        let (commands, _) = engine.frame((100., 100.), 0., None).unwrap();
        let text: String = commands
            .as_chunks::<STRIDE>()
            .0
            .iter()
            .filter_map(|c| char::from_u32(c[4] as u32))
            .collect();
        assert_eq!(
            text,
            "undefined,undefined,undefined,undefined,undefined".replace(' ', "")
        );
    }

    /// The commands of one frame of `scene`, as (op, args) rows, and its points.
    fn draw(scene: &str) -> (Vec<[f64; STRIDE]>, Vec<f64>) {
        let engine = Engine::new(scene, COLORS, 22.).unwrap();
        let (commands, points) = engine.frame((800., 600.), 1., None).unwrap();
        (commands.as_chunks::<STRIDE>().0.to_vec(), points)
    }

    #[test]
    fn a_path_built_once_draws_where_the_transform_puts_it() {
        // A unit square built at the top of the file, drawn twice: moved and
        // scaled, then turned a quarter around the same spot.
        let (commands, points) = draw(
            "const SQUARE = path().rect(0, 0, 1, 1);
             SCENES.x = { draw(g) {
               g.save(); g.translate(100, 50); g.scale(10); SQUARE.fill('#fff'); g.restore();
               g.translate(100, 50); g.rotate(Math.PI / 2); g.scale(10); SQUARE.stroke('#fff', 1, 2);
             } }",
        );
        assert_eq!(commands.len(), 2);
        let pairs = |c: &[f64; STRIDE]| {
            let (start, count) = (c[1] as usize, c[2] as usize);
            points[start * 2..(start + count) * 2]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| [p[0].round(), p[1]])
                .map(|p| [p[0], if p[0].is_nan() { p[1] } else { p[1].round() }])
                .collect::<Vec<_>>()
        };
        let fill = pairs(&commands[0]);
        assert_eq!((commands[0][0], commands[0][3]), (7., 0.), "a fill");
        assert_eq!(
            &fill[..4],
            &[[100., 50.], [110., 50.], [110., 60.], [100., 60.]]
        );
        assert!(fill[4][0].is_nan() && fill[4][1] == 1., "the square closes");
        let stroke = pairs(&commands[1]);
        assert_eq!(commands[1][3], 20., "the width scales with the shape");
        assert_eq!(&stroke[..3], &[[100., 50.], [100., 60.], [90., 60.]]);
    }

    #[test]
    fn shapes_under_a_turn_become_outlines_and_plain_ones_stay_quads() {
        let (commands, _) = draw(
            "SCENES.x = { draw(g) {
               g.rect(0, 0, 10, 10, '#fff'); g.ellipse(5, 5, 4, 2, '#fff');
               g.translate(20, 0); g.rect(0, 0, 10, 10, '#fff');
               g.rotate(0.3); g.rect(0, 0, 10, 10, '#fff'); g.ellipse(5, 5, 4, 2, '#fff');
             } }",
        );
        let ops: Vec<f64> = commands.iter().map(|c| c[0]).collect();
        assert_eq!(ops, [1., 2., 1., 7., 7.]);
        assert_eq!(
            commands[2][1], 20.,
            "a translated rect is still a quad, moved"
        );
    }

    #[test]
    fn noise_is_smooth_and_the_transform_resets_every_frame() {
        let probe = "SCENES.x = { draw(g) {
               const a = noise(1.30, 2.70, 0.5), b = noise(1.31, 2.70, 0.5);
               const far = [0, 1, 2, 3, 4, 5, 6, 7].map(i => noise(i * 3.7, 1.1));
               const spread = Math.max(...far) - Math.min(...far);
               g.rect(0, 0, Math.abs(a - b) < 0.05 ? 1 : 0, spread > 0.3 ? 1 : 0, '#fff');
               g.translate(500, 500);
             } }";
        let engine = Engine::new(probe, COLORS, 22.).unwrap();
        for _ in 0..2 {
            let (commands, _) = engine.frame((800., 600.), 0., None).unwrap();
            // Nearby inputs stay close, distant ones vary; a frame's
            // translate doesn't carry into the next (x stays 0).
            assert_eq!(&commands[..5], &[1., 0., 0., 1., 1.]);
        }
    }

    #[test]
    fn pixel_art_draws_one_crisp_rect_per_run_and_mirrors() {
        let (commands, _) = draw(
            "const ART = pixels(['aab.', '.a  '], { a: '#ff0000', b: '#00ff00' });
             SCENES.x = { draw(g) { g.pixels(ART, 10.4, 20.6, 3); g.pixels(ART, 0, 0, 2, 1, true); } }",
        );
        let rects: Vec<[f64; 5]> = commands
            .iter()
            .map(|c| [c[1], c[2], c[3], c[4], c[6]])
            .collect();
        let (red, green) = (f64::from(0xff0000), f64::from(0x00ff00));
        assert_eq!(
            rects,
            [
                // Snapped to (10, 21); "aa" is one rect, see-through skipped.
                [10., 21., 6., 3., red],
                [16., 21., 3., 3., green],
                [13., 24., 3., 3., red],
                // Mirrored in its 4-pixel width.
                [4., 0., 4., 2., red],
                [2., 0., 2., 2., green],
                [4., 2., 2., 2., red],
            ]
        );
    }
}
