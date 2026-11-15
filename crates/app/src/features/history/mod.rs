//! Commit History: a branch's commits as a graph, with their authors, refs
//! and changes, in a pane of its own. Read-only.
//!
//! `git::history` reads the commits; `gentle_core::git::log` parses them and
//! lays out the graph's lanes; `graph` paints them; `view` renders the pane.

mod graph;
mod view;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::*;

use crate::features::git::{self, Commit, Graph, History};

/// Commits read at first, and how many more "Earlier history" adds.
const PAGE: usize = 200;

/// One row of the list.
#[derive(Clone, Copy)]
enum Line {
    /// The work tree has changes on top of HEAD.
    Uncommitted,
    /// A commit by its index in the loaded history.
    Commit(usize),
    /// Older commits not loaded yet, by count.
    Earlier(usize),
}

pub struct HistoryView {
    toplevel: PathBuf,
    /// The branch asked for; `None` follows HEAD.
    branch: Option<String>,
    limit: usize,
    /// `None` until the first read, `Some(None)` outside a repository.
    history: Option<Option<History>>,
    commits: Rc<Vec<Commit>>,
    graph: Rc<Graph>,
    lines: Rc<Vec<Line>>,
    selected: Option<String>,
    loading: bool,
    /// Wall clock of the last read, Unix ms, for relative times.
    now_ms: i64,
    scroll: UniformListScrollHandle,
    /// The pane's width in the last frame: narrow panes drop columns.
    width: Rc<Cell<f32>>,
    focus: FocusHandle,
    _load: Task<()>,
}

impl Focusable for HistoryView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl HistoryView {
    /// `cwd` is any folder inside the repository.
    pub fn new(cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            toplevel: cwd,
            branch: None,
            limit: PAGE,
            history: None,
            commits: Rc::new(Vec::new()),
            graph: Rc::new(Graph::default()),
            lines: Rc::new(Vec::new()),
            selected: None,
            loading: false,
            now_ms: 0,
            scroll: UniformListScrollHandle::new(),
            width: Rc::default(),
            focus: cx.focus_handle(),
            _load: Task::ready(()),
        };
        this.reload(cx);
        this
    }

    /// The tab and pane label: `History · <repository>`.
    pub fn title(&self) -> SharedString {
        match self.toplevel.file_name() {
            Some(name) => format!("History · {}", name.to_string_lossy()).into(),
            None => "History".into(),
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let cwd = self.toplevel.clone();
        let branch = self.branch.clone();
        let limit = self.limit;
        self.loading = true;
        self._load = cx.spawn(async move |this, cx| {
            let history = cx
                .background_executor()
                .spawn(async move {
                    let toplevel = git::toplevel(&cwd)?;
                    git::history(&toplevel, branch.as_deref(), limit).map(|h| (toplevel, h))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.apply(history);
                cx.notify();
            });
        });
        cx.notify();
    }

    fn apply(&mut self, read: Option<(PathBuf, History)>) {
        self.now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let Some((toplevel, mut history)) = read else {
            self.history = Some(None);
            self.commits = Rc::new(Vec::new());
            self.graph = Rc::new(Graph::default());
            self.lines = Rc::new(Vec::new());
            return;
        };
        self.toplevel = toplevel;
        let graph = gentle_core::git::log::layout(&history.commits);
        // The list reads the commits from `self.commits`; the rest of the
        // read stays in `self.history`.
        let commits = std::mem::take(&mut history.commits);
        let mut lines = Vec::with_capacity(commits.len() + 2);
        if history.dirty && !commits.is_empty() {
            lines.push(Line::Uncommitted);
        }
        lines.extend((0..commits.len()).map(Line::Commit));
        let earlier = history.total.saturating_sub(commits.len());
        if earlier > 0 {
            lines.push(Line::Earlier(earlier));
        }
        if !self
            .selected
            .as_ref()
            .is_some_and(|hash| commits.iter().any(|c| &c.hash == hash))
        {
            self.selected = None;
        }
        self.commits = Rc::new(commits);
        self.graph = Rc::new(graph);
        self.lines = Rc::new(lines);
        self.history = Some(Some(history));
    }

    fn show_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        self.branch = Some(branch);
        self.limit = PAGE;
        self.selected = None;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.reload(cx);
    }

    fn load_earlier(&mut self, cx: &mut Context<Self>) {
        if !self.loading {
            self.limit += PAGE;
            self.reload(cx);
        }
    }

    fn select(&mut self, hash: String, cx: &mut Context<Self>) {
        self.selected = if self.selected.as_ref() == Some(&hash) {
            None
        } else {
            Some(hash)
        };
        cx.notify();
    }

    fn copy_selected(&self, cx: &mut Context<Self>) {
        if let Some(hash) = &self.selected {
            cx.write_to_clipboard(ClipboardItem::new_string(hash.clone()));
        }
    }
}
