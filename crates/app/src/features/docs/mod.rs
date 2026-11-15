//! Architecture docs: one screen to set up and drive the `architecture-docs`
//! skill (ADRs, C4, data model, states, flows… kept in sync with the code)
//! for the workspace's folder. It installs what is missing — the skill into
//! Gentle Shell, the diagram renderer into the app — lists the documents with
//! how current each is, and starts agent tabs that write or update them.
//!
//! The agent does the writing; this screen only asks. Document standing
//! comes from the skill's own `docs-check` script (`gentle_core::docs`).

mod view;

use std::fs;
use std::path::{Path, PathBuf};

use gentle_core::docs::{Report, Standing, title};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

/// The skill, and where Gentle Shell installs it from.
pub const SKILL: &str = "architecture-docs";
const SKILL_SOURCE: &str = "git:github.com/ismaelosuna7824/gentle-architecture-docs";

pub enum DocsEvent {
    Close,
    /// Start an agent tab in the folder with this prompt.
    Run(String),
    /// Show this document.
    Open(PathBuf),
}

/// One document under `docs/`.
#[derive(Clone)]
struct Doc {
    path: PathBuf,
    /// Relative to the folder, `/`-separated (as the report names it).
    relative: String,
    title: String,
    standing: Option<Standing>,
    notes: Vec<String>,
}

/// What the agent can be asked for. `Feature` and `Decision` need a name,
/// typed in the row before it runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ask {
    Project,
    Feature,
    Decision,
    Update,
}

impl Ask {
    const ALL: [Ask; 4] = [Ask::Project, Ask::Feature, Ask::Decision, Ask::Update];

    fn command(self) -> &'static str {
        match self {
            Ask::Project => "init",
            Ask::Feature => "feature",
            Ask::Decision => "adr",
            Ask::Update => "update",
        }
    }

    fn needs_name(self) -> bool {
        matches!(self, Ask::Feature | Ask::Decision)
    }
}

#[derive(Clone)]
enum SkillState {
    Missing,
    Installing,
    /// Installed from here; agent tabs started from now on have it.
    JustInstalled,
    Ready(PathBuf),
    Failed(String),
}

pub struct DocsView {
    root: PathBuf,
    skill: SkillState,
    docs: Option<Vec<Doc>>,
    /// Why the documents' standing is unknown.
    check_note: Option<String>,
    head: Option<String>,
    /// The row asking for a name, open.
    asking: Option<Ask>,
    name: Entity<InputState>,
    _load: Task<()>,
    _install: Task<()>,
    _renderer: Subscription,
    _name: Subscription,
}

impl EventEmitter<DocsEvent> for DocsView {}

impl DocsView {
    /// `skill_dir`: where an open agent tab found the skill, if one did.
    pub fn new(
        root: PathBuf,
        skill_dir: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        let name_sub = cx.subscribe(&name, |this, _, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.submit(cx);
            }
        });
        let mut this = Self {
            root,
            skill: skill_dir.map_or(SkillState::Missing, SkillState::Ready),
            docs: None,
            check_note: None,
            head: None,
            asking: None,
            name,
            _load: Task::ready(()),
            _install: Task::ready(()),
            _renderer: cx.observe_global::<crate::features::files::DiagramRendererState>(
                |_, cx| cx.notify(),
            ),
            _name: name_sub,
        };
        this.reload(cx);
        this
    }

    /// An agent tab now reports where the skill lives.
    pub fn set_skill_dir(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(dir) = dir else { return };
        if matches!(&self.skill, SkillState::Ready(known) if *known == dir) {
            return;
        }
        self.skill = SkillState::Ready(dir);
        self.reload(cx);
    }

    fn skill_ready(&self) -> bool {
        matches!(self.skill, SkillState::Ready(_) | SkillState::JustInstalled)
    }

    /// List `docs/` and, with the skill at hand, check each document.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let script = match &self.skill {
            SkillState::Ready(dir) => Some(dir.join("scripts").join("docs-check.mjs")),
            _ => None,
        };
        self._load = cx.spawn(async move |this, cx| {
            let (docs, report, note) = cx
                .background_executor()
                .spawn(async move { load(&root, script.as_deref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.head = report.as_ref().and_then(|r| r.head.clone());
                this.docs = Some(docs);
                this.check_note = note;
                cx.notify();
            });
        });
    }

    fn install_skill(&mut self, cx: &mut Context<Self>) {
        self.skill = SkillState::Installing;
        let root = self.root.clone();
        self._install = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::features::setup::run_gentle_shell(&root, &["install", SKILL_SOURCE])
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skill = match result {
                    Ok(_) => SkillState::JustInstalled,
                    Err(output) => SkillState::Failed(
                        output
                            .lines()
                            .rev()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or("gentle-shell install failed")
                            .trim()
                            .to_string(),
                    ),
                };
                cx.notify();
            });
        });
        cx.notify();
    }

    /// A click on an action: run it, or open its row for the name it needs.
    fn ask(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        if !ask.needs_name() {
            cx.emit(DocsEvent::Run(format!("/skill:{SKILL} {}", ask.command())));
            return;
        }
        let placeholder = match ask {
            Ask::Feature => "Feature name, e.g. payments",
            _ => "The decision, e.g. Keep balances in minor units",
        };
        self.asking = Some(ask);
        self.name.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.set_placeholder(placeholder, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Run the open row with the name typed in it.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(ask) = self.asking else { return };
        let name = self.name.read(cx).value().trim().to_string();
        if !name.is_empty() {
            cx.emit(DocsEvent::Run(format!(
                "/skill:{SKILL} {} {name}",
                ask.command()
            )));
        }
    }

    fn cancel_ask(&mut self, cx: &mut Context<Self>) {
        self.asking = None;
        cx.notify();
    }
}

/// Blocking: the documents under `<root>/docs` with their standing from
/// `script` (the skill's `docs-check.mjs`), and why there is none.
fn load(root: &Path, script: Option<&Path>) -> (Vec<Doc>, Option<Report>, Option<String>) {
    let dir = root.join("docs");
    let mut paths = Vec::new();
    markdown_files(&dir, &mut paths);
    paths.sort();
    let (report, note) = match script {
        None => (
            None,
            Some("Install the docs skill to see which documents are current.".to_string()),
        ),
        Some(_) if paths.is_empty() => (None, None),
        Some(script) => match check(root, script) {
            Ok(report) => (Some(report), None),
            Err(error) => (None, Some(error)),
        },
    };
    let docs = paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let text = fs::read_to_string(&path).unwrap_or_default();
            let title = title(&text)
                .map(str::to_string)
                .or_else(|| {
                    text.lines()
                        .find_map(|l| l.strip_prefix("# ").map(|t| t.trim().to_string()))
                })
                .unwrap_or_else(|| relative.clone());
            let (standing, notes) = match &report {
                Some(report) => (
                    Some(report.standing(&relative)),
                    report.about(&relative).map(|f| f.message.clone()).collect(),
                ),
                None => (None, Vec::new()),
            };
            Doc {
                path,
                relative,
                title,
                standing,
                notes,
            }
        })
        .collect();
    (docs, report, note)
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// Blocking: `node <script> --json` in `root`.
fn check(root: &Path, script: &Path) -> Result<Report, String> {
    let node = pi_rpc::find_program("node").ok_or("Node.js wasn't found to run the docs check.")?;
    let out = pi_rpc::command(&node)
        .arg(script)
        .arg("--json")
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't run the docs check: {e}"))?;
    Report::parse(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
        let error = String::from_utf8_lossy(&out.stderr);
        format!(
            "The docs check gave no report{}",
            error
                .lines()
                .next()
                .map(|l| format!(": {l}"))
                .unwrap_or_default()
        )
    })
}
