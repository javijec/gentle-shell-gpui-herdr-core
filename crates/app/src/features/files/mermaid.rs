//! Mermaid diagrams in the Markdown preview. Each ` ```mermaid ` block is
//! drawn as an SVG image by merman's CLI, the *diagram renderer*: not part of
//! the app, it is downloaded the first time someone asks for it (see
//! [`install`]). A block shows as its source until it is drawn (off the UI
//! thread), and stays source with the error under it when it can't be.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use gentle_core::markdown::mermaid_blocks;
use gpui_kit::{
    App, BorrowAppContext as _, Global, Image, ImageFormat, ImageSource, SharedUri, Task,
};

/// How a document's images name a drawn diagram: `mermaid:<key>`.
const SCHEME: &str = "mermaid:";
/// Drawn diagrams kept per preview; a long typing session makes many.
const KEEP: usize = 64;

/// The diagram renderer, app-wide: previews and the Docs screen follow it.
#[derive(Clone, Debug, PartialEq)]
pub enum Renderer {
    Missing,
    Installing(String),
    Ready(PathBuf),
    Failed(String),
}

pub struct RendererState {
    renderer: Renderer,
    _install: Task<()>,
}

impl Global for RendererState {}

pub(super) fn init(cx: &mut App) {
    let renderer =
        crate::features::setup::diagram_renderer().map_or(Renderer::Missing, Renderer::Ready);
    cx.set_global(RendererState {
        renderer,
        _install: Task::ready(()),
    });
}

/// The diagram renderer's state.
pub fn renderer(cx: &App) -> &Renderer {
    &cx.global::<RendererState>().renderer
}

/// Download and install the diagram renderer; observers of the global (the
/// previews, the Docs screen) redraw as it goes.
pub fn install(cx: &mut App) {
    if matches!(renderer(cx), Renderer::Installing(_) | Renderer::Ready(_)) {
        return;
    }
    let set = |renderer: Renderer, cx: &mut App| {
        cx.update_global::<RendererState, _>(|state, _| state.renderer = renderer);
    };
    set(Renderer::Installing("Starting the download…".into()), cx);
    let (tx, rx) = async_channel::unbounded::<String>();
    let task = cx.spawn(async move |cx| {
        let work = cx.background_executor().spawn(async move {
            crate::features::setup::install_diagram_renderer(&move |line| {
                let _ = tx.try_send(line);
            })
        });
        while let Ok(line) = rx.recv().await {
            let line = line.trim().to_string();
            cx.update(|cx| set(Renderer::Installing(line), cx));
        }
        let done = work.await;
        cx.update(|cx| {
            set(
                match done {
                    Ok(path) => Renderer::Ready(path),
                    Err(error) => Renderer::Failed(error),
                },
                cx,
            )
        });
    });
    cx.global_mut::<RendererState>()._install = task;
}

#[derive(Clone)]
enum Drawing {
    Pending,
    Ready(Arc<Image>),
    Failed(String),
}

/// The diagrams of one preview, by source and theme.
#[derive(Default)]
pub(super) struct Diagrams(Mutex<HashMap<u64, Drawing>>);

/// A diagram to draw: its key, its source, the theme and the renderer.
pub(super) struct Job {
    key: u64,
    source: String,
    dark: bool,
    cli: PathBuf,
}

/// What [`Diagrams::prepare`] gives the preview.
pub(super) struct Prepared {
    /// The document as shown.
    pub text: String,
    /// Mermaid blocks in it.
    pub blocks: usize,
    /// Diagrams still to draw (only with a renderer).
    pub jobs: Vec<Job>,
}

impl Diagrams {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Drawing>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `text` as the preview shows it: drawn diagrams as images, the others
    /// as their source (with the error under a failed one). Without a
    /// renderer (`cli`) nothing is drawn.
    pub(super) fn prepare(&self, text: &str, dark: bool, cli: Option<&PathBuf>) -> Prepared {
        let blocks = mermaid_blocks(text);
        let mut prepared = Prepared {
            text: String::with_capacity(text.len()),
            blocks: blocks.len(),
            jobs: Vec::new(),
        };
        let Some(cli) = cli.filter(|_| !blocks.is_empty()) else {
            prepared.text.push_str(text);
            return prepared;
        };
        let mut cache = self.lock();
        let mut at = 0;
        for block in blocks {
            let source = &text[block.source.clone()];
            let key = key(source, dark);
            let out = &mut prepared.text;
            out.push_str(&text[at..block.range.start]);
            match cache.get(&key) {
                Some(Drawing::Ready(_)) => {
                    out.push_str(&format!("![Mermaid diagram]({SCHEME}{key})\n"));
                }
                Some(Drawing::Failed(error)) => {
                    out.push_str(&text[block.range.clone()]);
                    out.push_str(&format!(
                        "\n> Mermaid couldn't draw this diagram: {error}\n"
                    ));
                }
                Some(Drawing::Pending) => out.push_str(&text[block.range.clone()]),
                None => {
                    out.push_str(&text[block.range.clone()]);
                    cache.insert(key, Drawing::Pending);
                    prepared.jobs.push(Job {
                        key,
                        source: source.to_string(),
                        dark,
                        cli: cli.clone(),
                    });
                }
            }
            at = block.range.end;
        }
        prepared.text.push_str(&text[at..]);
        if cache.len() > KEEP {
            // Edits leave old versions behind; keep only what is pending.
            cache.retain(|_, d| matches!(d, Drawing::Pending));
        }
        prepared
    }

    /// Store what drawing `job` gave.
    pub(super) fn finish(&self, job: &Job, drawn: Result<Vec<u8>, String>) {
        let drawing = match drawn {
            Ok(svg) => Drawing::Ready(Arc::new(Image::from_bytes(ImageFormat::Svg, svg))),
            Err(error) => Drawing::Failed(error),
        };
        self.lock().insert(job.key, drawing);
    }

    /// The image for a `mermaid:<key>` URI, if it is one.
    pub(super) fn image(&self, uri: &SharedUri) -> Option<ImageSource> {
        let key: u64 = uri.strip_prefix(SCHEME)?.parse().ok()?;
        match self.lock().get(&key)? {
            Drawing::Ready(image) => Some(ImageSource::Image(image.clone())),
            _ => None,
        }
    }
}

fn key(source: &str, dark: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    hasher.finish()
}

/// The sans font the renderer measures text with: the one GPUI's SVG
/// renderer will draw it with, so labels fit their boxes. Found once
/// (loading the font database takes a moment).
static FONT: LazyLock<Option<(String, PathBuf)>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let families = [
        "Arial",
        "Helvetica",
        "Segoe UI",
        "DejaVu Sans",
        "Liberation Sans",
    ]
    .map(fontdb::Family::Name);
    let face = families
        .iter()
        .chain([&fontdb::Family::SansSerif])
        .find_map(|family| {
            let id = db.query(&fontdb::Query {
                families: std::slice::from_ref(family),
                ..fontdb::Query::default()
            })?;
            let face = db.face(id)?;
            let path = match &face.source {
                fontdb::Source::File(path) => path.clone(),
                fontdb::Source::SharedFile(path, _) => path.clone(),
                fontdb::Source::Binary(_) => return None,
            };
            Some((face.families.first()?.0.clone(), path))
        })?;
    Some(face)
});

/// Draw `job` as SVG with the renderer. Tens of milliseconds: call it off
/// the UI thread.
pub(super) fn draw(job: &Job) -> Result<Vec<u8>, String> {
    let mut init = vec![format!(
        "\"theme\": \"{}\"",
        if job.dark { "dark" } else { "default" }
    )];
    let mut command = pi_rpc::command(&job.cli);
    command.args([
        "render",
        "-",
        "-o",
        "-",
        "-f",
        "svg",
        "--svg-pipeline",
        "resvg-safe",
        "-q",
    ]);
    if let Some((family, path)) = FONT.as_ref() {
        command.arg("--font-file").arg(path);
        init.push(format!("\"fontFamily\": \"{family}\""));
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't start the diagram renderer: {e}"))?;
    let source = format!("%%{{init: {{{}}}}}%%\n{}", init.join(", "), job.source);
    child
        .stdin
        .take()
        .ok_or("the diagram renderer has no input")?
        .write_all(source.as_bytes())
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() && !output.stdout.is_empty() {
        Ok(output.stdout)
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if error.is_empty() {
            "the diagram renderer gave no drawing".to_string()
        } else {
            error
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Diagrams, draw};

    const DOC: &str = "# Flow\n\n```mermaid\nflowchart LR\n  a[Start] --> b[End]\n```\n\nafter\n";

    #[test]
    fn without_a_renderer_diagrams_stay_source_and_nothing_is_drawn() {
        let prepared = Diagrams::default().prepare(DOC, true, None);
        assert_eq!(prepared.text, DOC);
        assert_eq!(prepared.blocks, 1);
        assert!(prepared.jobs.is_empty());
    }

    #[test]
    fn a_diagram_shows_as_source_until_drawn_then_as_an_image() {
        let diagrams = Diagrams::default();
        let cli = PathBuf::from("merman-cli");
        let prepared = diagrams.prepare(DOC, true, Some(&cli));
        assert_eq!(prepared.text, DOC);
        assert_eq!(prepared.jobs.len(), 1);
        // Asked again while drawing: still the source, no second job.
        assert!(diagrams.prepare(DOC, true, Some(&cli)).jobs.is_empty());

        let svg = b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'/>".to_vec();
        diagrams.finish(&prepared.jobs[0], Ok(svg));
        let shown = diagrams.prepare(DOC, true, Some(&cli)).text;
        assert!(shown.starts_with("# Flow\n\n![Mermaid diagram](mermaid:"));
        assert!(shown.ends_with(")\n\nafter\n"));
        let uri = shown.split('(').nth(1).unwrap().split(')').next().unwrap();
        assert!(diagrams.image(&uri.to_string().into()).is_some());
        // The other theme is another drawing.
        assert_eq!(diagrams.prepare(DOC, false, Some(&cli)).jobs.len(), 1);
    }

    #[test]
    fn a_diagram_that_cannot_be_drawn_keeps_its_source_and_says_why() {
        let doc = "```mermaid\nnot a diagram at all\n```\n";
        let diagrams = Diagrams::default();
        let cli = PathBuf::from("/nonexistent/merman-cli");
        let prepared = diagrams.prepare(doc, false, Some(&cli));
        let drawn = draw(&prepared.jobs[0]);
        assert!(drawn.is_err());
        diagrams.finish(&prepared.jobs[0], drawn);
        let shown = diagrams.prepare(doc, false, Some(&cli)).text;
        assert!(shown.starts_with(doc));
        assert!(shown.contains("> Mermaid couldn't draw this diagram:"));
    }
}
