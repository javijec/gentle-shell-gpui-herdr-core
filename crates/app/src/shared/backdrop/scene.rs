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

/// One computed frame: `STRIDE` numbers per command, for a window of `size`.
pub struct Frame {
    pub commands: Vec<f64>,
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
fn pace(cost: Duration, floor: Duration) -> Duration {
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

    /// The commands for a `size` window at scene time `time`.
    pub fn frame(&self, size: (f32, f32), time: f64) -> Result<Vec<f64>, String> {
        self.arm(FRAME_BUDGET);
        self.context.with(|ctx| {
            let api: Object = ctx.globals().get("__api").map_err(|e| e.to_string())?;
            let frame: Function = api.get("frame").map_err(|e| e.to_string())?;
            let count = frame
                .call::<_, f64>((size.0, size.1, time, self.cell))
                .map_err(|e| js_error(&ctx, e))? as usize;
            let out: Function = api.get("out").map_err(|e| e.to_string())?;
            let buffer: TypedArray<f64> = out.call(()).map_err(|e| js_error(&ctx, e))?;
            let bytes = buffer.arraybuffer().map_err(|e| e.to_string())?;
            // SAFETY: no JS runs while `bytes` is borrowed, so nothing can
            // detach or resize the buffer under it; it's copied right away.
            let bytes = unsafe { bytes.as_bytes() }.ok_or("the frame buffer is gone")?;
            let used = (count * STRIDE * 8).min(bytes.len());
            Ok(bytes[..used]
                .as_chunks::<8>()
                .0
                .iter()
                .map(|b| f64::from_ne_bytes(*b))
                .collect())
        })
    }
}

fn run(shared: &Shared, setup: Setup) -> Result<(), String> {
    let engine = Engine::new(&setup.code, &setup.colors, setup.cell)?;
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
        match engine.frame(size, time) {
            Ok(commands) => {
                failures = 0;
                if drawn.is_none() {
                    super::report(&setup.theme, None);
                }
                drawn = Some(size);
                *shared.frame.lock().unwrap() = Some(Arc::new(Frame { commands, size }));
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
                let commands = engine
                    .frame((1440., 900.), t)
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
        assert!(engine.frame((800., 600.), 0.).is_err());
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
        let commands = engine.frame((100., 100.), 0.).unwrap();
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
}
