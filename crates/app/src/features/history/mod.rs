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
use std::time::Duration;

use serde_json::{Value, json};

use gpui_kit::*;

use crate::features::git::{self, Commit, Graph, History};
use crate::features::session::SessionView;

/// Remote history is a read-only query on this same agent's existing Herdr
/// RPC bridge. Linux paths remain POSIX strings, never passed to Windows Git.
#[derive(Clone)]
struct RemoteHistory {
    machine: String,
    root: String,
    session: Entity<SessionView>,
}

/// Decode the same structured Git history that the native History screen
/// consumes, with explicit bounds and no fallback to the local repository.
fn parse_remote_history(data: &Value, root: &str) -> Option<History> {
    let toplevel = data["toplevel"].as_str()?;
    if !toplevel.starts_with('/') || toplevel.contains('\\')
        || (toplevel != root && !toplevel.starts_with(&(root.to_owned() + "/")))
    {
        return None;
    }
    let raw = data["log"].as_str()?;
    if raw.len() > 384 * 1024 {
        return None;
    }
    let commits = gentle_core::git::log::parse_log(raw);
    if commits.len() > 800 {
        return None;
    }
    let total = usize::try_from(data["total"].as_u64()?).ok()?;
    let branches = data["branches"].as_array()?
        .iter().map(|value| {
            let name = value.as_str()?;
            (!name.is_empty() && name.len() <= 512 &&
                !name.chars().any(char::is_control))
                .then(|| name.to_owned())
        }).collect::<Option<Vec<_>>>()?;
    if branches.len() > 120 {
        return None;
    }
    let current = data["current"].as_str()
        .map(str::to_owned);
    let branch = data["branch"].as_str()
        .map(str::to_owned);
    Some(History {
        commits, total, current, branch, branches,
        dirty: data["dirty"].as_bool()?,
    })
}

#[cfg(test)]
mod remote_history_tests {
    use super::parse_remote_history;
    use serde_json::json;

    #[test]
    fn remote_history_is_root_scoped_and_uses_the_native_model() {
        let data = json!({
            "toplevel": "/srv/workspaces/project/repo",
            "log": "",
            "total": 0,
            "current": "main",
            "branch": "main",
            "branches": ["main", "feature/test"],
            "dirty": false
        });
        let parsed = parse_remote_history(&data, "/srv/workspaces/project").unwrap();
        assert_eq!(parsed.branch.as_deref(), Some("main"));
        assert_eq!(parsed.branches.len(), 2);
        assert_eq!(parsed.total, 0);
        assert!(parse_remote_history(&data, "/srv/workspaces/other").is_none());
        let mut escaped = data.clone();
        escaped["toplevel"] = json!("/srv/workspaces/project2");
        assert!(parse_remote_history(&escaped, "/srv/workspaces/project").is_none());
        escaped["toplevel"] = json!("C:\\Programacion\\repo");
        assert!(parse_remote_history(&escaped, "/srv/workspaces/project").is_none());
    }

    #[test]
    fn remote_history_rejects_invalid_refs_and_overlong_records() {
        let mut data = json!({
            "toplevel": "/srv/p",
            "log": "",
            "total": 0,
            "current": null,
            "branch": null,
            "branches": ["main"],
            "dirty": false
        });
        data["branches"] = json!(["bad\nbranch"]);
        assert!(parse_remote_history(&data, "/srv/p").is_none());
        data["branches"] = json!(["main"]);
        data["log"] = json!("x".repeat(384 * 1024 + 1));
        assert!(parse_remote_history(&data, "/srv/p").is_none());
    }
}

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
    remote: Option<RemoteHistory>,
    error: Option<String>,
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
            remote: None,
            error: None,
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

    /// Same graph, header and branch picker with a remote Herdr source.
    pub fn new_remote(
        machine: String,
        root: String,
        session: Entity<SessionView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            toplevel: PathBuf::new(),
            remote: Some(RemoteHistory { machine, root, session }),
            error: None,
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

    pub(super) fn folder_label(&self) -> String {
        if let Some(remote) = &self.remote {
            return format!("{} · {}", remote.machine, remote.root);
        }
        self.toplevel.file_name().map_or_else(
            String::new, |name| name.to_string_lossy().into_owned(),
        )
    }

    /// The tab and pane label: `History · <repository>`.
    pub fn title(&self) -> SharedString {
        format!("History · {}", self.folder_label()).into()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        if let Some(remote) = self.remote.clone() {
            let branch = self.branch.clone();
            let limit = self.limit;
            self.loading = true;
            self._load = cx.spawn(async move |this, cx| {
                // A restored History pane can open before its owning Pi
                // connects. Wait for this same agent instead of ever using
                // the Windows Git installation as a fallback.
                let mut process = None;
                for _ in 0..45 {
                    process = this.update(cx, |_, cx| {
                        remote.session.read(cx).remote_fs_process()
                    }).ok().flatten();
                    if process.is_some() { break; }
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                }
                let reply = match process {
                    Some(process) => process.request(json!({
                        "type": "herdr_git_history",
                        "root": remote.root,
                        "branch": branch,
                        "limit": limit,
                    })).await.map_err(|error| error.to_string()),
                    None => Err("The original remote agent is not connected".to_owned()),
                };
                let (history, error) = match reply {
                    Ok(response) => {
                        let parsed = parse_remote_history(&response.data, &remote.root);
                        let error = parsed.is_none()
                            .then(|| "Invalid remote Git History response".to_owned());
                        (parsed, error)
                    }
                    Err(error) => (None, Some(format!(
                        "Remote History unavailable on this agent: {error}. Open a new remote agent if it uses an older bridge."
                    ))),
                };
                let _ = this.update(cx, |this, cx| {
                    this.loading = false;
                    this.error = error;
                    this.apply(history.map(|history| (PathBuf::new(), history)));
                    cx.notify();
                });
            });
            cx.notify();
            return;
        }
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
