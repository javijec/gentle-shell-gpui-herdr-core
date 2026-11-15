//! The switcher (⌘K), after Tern's: one search field over segmented
//! categories — the app's screens and pane commands plus every Gentle Shell
//! slash command the backend reports, the window's workspaces (switch,
//! rename, delete, create), files under the workspace root and git branches.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use gentle_core::commands::Screen;
use gentle_core::fuzzy::{FileEntry, Needle};

mod commands;
mod keyboard;
mod rows;
mod sources;
mod view;

use commands::{command_row, screen_rows};
pub use sources::switch_branch;
use sources::{list_branches, walk_files};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteAction {
    Providers,
    Profiles,
    Sessions,
    Machines,
    Usage,
    Memory,
    Docs,
    History,
    Devices,
    Settings,
    /// The onboarding screen: dependency checks and installs.
    Setup,
    /// The first-run welcome screen, from its intro.
    Welcome,
    Agents,
    NewSession,
    /// A new tab with an agent pane.
    NewTab,
    NewTerminalTab,
    /// A new pane of the focused kind, tiled Hyprland-style.
    TilePane,
    SplitRight,
    SplitDown,
    /// Split the focused pane with an agent / terminal beside it.
    NewAgentPane,
    NewTerminalPane,
    ClosePane,
    ReopenClosed,
    ToggleSidebar,
    /// Send `/name` as a prompt in the active agent pane.
    Command(String),
    /// A Gentle Shell command whose UI is terminal-only, opened natively.
    Screen(Screen),
    SwitchWorkspace(usize),
    RenameWorkspace {
        index: usize,
        name: String,
    },
    DeleteWorkspace(usize),
    /// A new workspace named `name`, rooted at the current folder or, with
    /// `pick_folder`, at one the user chooses (named after it when `name` is empty).
    CreateWorkspace {
        name: String,
        pick_folder: bool,
    },
    OpenFile(PathBuf),
    /// Remote Linux paths are POSIX strings, never Windows paths. Include
    /// the originating machine/root to reject stale palette selections.
    OpenRemoteFile { machine: String, root: String, terminal: String, path: String },
    /// Asks for confirmation before running `git switch`.
    SwitchBranch(String),
    SwitchRemoteBranch { machine: String, root: String, terminal: String, branch: String },
}

pub enum PaletteEvent {
    Run(PaletteAction),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    All,
    Commands,
    Workspaces,
    Files,
    Branches,
}

const CATEGORIES: [(Category, &str); 5] = [
    (Category::All, "All"),
    (Category::Commands, "Commands"),
    (Category::Workspaces, "Workspaces"),
    (Category::Files, "Files"),
    (Category::Branches, "Branches"),
];

impl Category {
    fn placeholder(self) -> &'static str {
        match self {
            Category::All => "Search commands, workspaces, files…",
            Category::Commands => "Type a command or screen…",
            Category::Workspaces => "Find or create a workspace…",
            Category::Files => "Find a file…",
            Category::Branches => "Find a branch…",
        }
    }
}

/// A workspace as the switcher lists it.
#[derive(Clone, Debug)]
pub struct WorkspaceRow {
    /// Index in the window's workspace list.
    pub index: usize,
    pub name: String,
    pub tabs: usize,
    pub current: bool,
    pub activity: Activity,
}

/// What a workspace's agents are doing: how many work and how many wait on
/// you (an agent that waits isn't counted as working).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub working: usize,
    pub needs_you: usize,
}

#[derive(Clone)]
pub struct PaletteRemote {
    pub machine: String,
    pub root: String,
    pub terminal: String,
    pub process: Option<Arc<pi_rpc::RpcProcess>>,
}

pub struct PaletteInput {
    /// Slash commands of the active agent pane: `(name, description)`.
    pub commands: Vec<(String, String)>,
    pub workspaces: Vec<WorkspaceRow>,
    /// The Windows workspace root; not used for filesystem/branch queries
    /// when a remote source is present.
    pub root: PathBuf,
    pub remote: Option<PaletteRemote>,
    pub category: Category,
}

#[derive(Clone)]
struct Row {
    icon: IconName,
    title: String,
    meta: String,
    detail: String,
    group: &'static str,
    action: PaletteAction,
    /// Workspace rows get rename/delete buttons.
    workspace: Option<usize>,
    current: bool,
}

enum Line {
    Header(&'static str),
    Row(usize),
}

const MAX_FILES: usize = 20_000;

pub struct PaletteView {
    query: Entity<InputState>,
    category: Category,
    selected: usize,
    commands: Vec<Row>,
    workspaces: Vec<WorkspaceRow>,
    root: PathBuf,
    remote: Option<PaletteRemote>,
    remote_error: Option<String>,
    remote_truncated: bool,
    files: Option<Vec<FileEntry>>,
    branches: Option<Vec<(String, bool)>>,
    renaming: Option<(usize, Entity<InputState>)>,
    /// The query as last matched.
    needle: Needle,
    /// Matched rows and their display lines, recomputed by
    /// [`Self::refresh_rows`] when the query, category or sources change.
    rows: Vec<Row>,
    lines: Vec<Line>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<PaletteEvent> for PaletteView {}

impl PaletteView {
    pub fn new(input: PaletteInput, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(input.category.placeholder()));
        query.update(cx, |state, cx| state.focus(window, cx));
        let subscription = cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.selected = 0;
                this.scroll.scroll_to_item(0);
                this.refresh_rows(cx);
                cx.notify();
            }
        });
        let mut commands = screen_rows();
        if !crate::features::terminal::herdr_enabled() {
            // Hide Machines from the command palette in native/local mode.
            commands.retain(|row| row.action != PaletteAction::Machines);
        }
        commands.extend(input.commands.into_iter().filter_map(command_row));

        let tasks = if let Some(remote) = input.remote.clone() {
            vec![cx.spawn(async move |this, cx| {
                let result = match remote.process {
                    Some(process) => process.request(json!({
                        "type": "herdr_palette_snapshot",
                        "root": remote.root,
                    })).await.map_err(|error| error.to_string()),
                    None => Err("The selected remote agent is disconnected or its directory is unknown".to_owned()),
                };
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(response) => {
                            let parsed = sources::parse_remote_palette(&response.data);
                            match parsed {
                                Some((files, branches, truncated)) => {
                                    this.files = Some(files);
                                    this.branches = Some(branches);
                                    this.remote_truncated = truncated;
                                }
                                None => {
                                    this.files = Some(Vec::new());
                                    this.branches = Some(Vec::new());
                                    this.remote_error = Some(
                                        "The remote palette returned invalid data.".into()
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            this.files = Some(Vec::new());
                            this.branches = Some(Vec::new());
                            this.remote_error = Some(format!(
                                "Remote project indexing failed: {error}. Open a new agent if it uses an older bridge."
                            ));
                        }
                    }
                    this.refresh_rows(cx);
                    cx.notify();
                });
            })]
        } else {
            let root = input.root.clone();
            let files = cx.spawn(async move |this, cx| {
                let files = cx
                    .background_executor()
                    .spawn(async move {
                        walk_files(&root, MAX_FILES)
                            .into_iter()
                            .map(FileEntry::new)
                            .collect::<Vec<_>>()
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.files = Some(files);
                    this.refresh_rows(cx);
                    cx.notify();
                });
            });
            let root = input.root.clone();
            let branches = cx.spawn(async move |this, cx| {
                let branches = cx
                    .background_executor()
                    .spawn(async move { list_branches(&root) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.branches = Some(branches);
                    this.refresh_rows(cx);
                    cx.notify();
                });
            });
            vec![files, branches]
        };

        let mut this = Self {
            query,
            category: input.category,
            selected: 0,
            commands,
            workspaces: input.workspaces,
            root: input.root,
            remote: input.remote,
            remote_error: None,
            remote_truncated: false,
            files: None,
            branches: None,
            renaming: None,
            needle: Needle::default(),
            rows: Vec::new(),
            lines: Vec::new(),
            scroll: ScrollHandle::new(),
            _subscriptions: vec![subscription],
            _tasks: tasks,
        };
        this.refresh_rows(cx);
        this
    }

    pub fn category(&self) -> Category {
        self.category
    }

    pub fn set_category(
        &mut self,
        category: Category,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.category = category;
        self.selected = 0;
        self.renaming = None;
        self.scroll.scroll_to_item(0);
        self.query.update(cx, |state, cx| {
            state.set_placeholder(category.placeholder(), window, cx);
            state.focus(window, cx);
        });
        self.refresh_rows(cx);
        cx.notify();
    }

    /// The workspaces changed while the switcher is open (an agent started,
    /// finished or asked something): re-list them in place.
    pub fn set_workspaces(&mut self, workspaces: Vec<WorkspaceRow>, cx: &mut Context<Self>) {
        self.workspaces = workspaces;
        self.refresh_rows(cx);
        cx.notify();
    }
}
