//! Registers the terminal, file-viewer and history panes with the tiling
//! layer, and wires the file sidebar to the workspace.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;

use crate::features::files::FileView;
use crate::features::history::HistoryView;
use crate::features::terminal::TerminalView;
use crate::shared::pane::{PaneFactories, PaneItem, PaneKind};

/// The git work tree containing `path` (FileView::diff runs git there).
pub fn repo_root(path: &std::path::Path) -> PathBuf {
    let dir = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    pi_rpc::command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or_else(|| dir.to_path_buf())
}

pub fn register(cx: &mut App) {
    crate::features::files::init(cx);
    let terminal: crate::shared::pane::PaneFactory =
        Rc::new(|cwd: PathBuf, window: &mut Window, cx: &mut App| {
            let view = cx.new(|cx| TerminalView::new(cwd, window, cx));
            let focus = view.focus_handle(cx);
            let title_view = view.clone();
            PaneItem {
                kind: PaneKind::Terminal,
                view: view.into(),
                focus,
                title: Rc::new(move |cx: &App| title_view.read(cx).title()),
                unsaved: None,
            }
        });
    let file: crate::shared::pane::PaneFactory =
        Rc::new(|path: PathBuf, window: &mut Window, cx: &mut App| {
            let view = cx.new(|cx| FileView::open(path, window, cx));
            let focus = view.focus_handle(cx);
            let title_view = view.clone();
            PaneItem {
                kind: PaneKind::File,
                view: view.into(),
                focus,
                unsaved: {
                    let view = title_view.clone();
                    Some(Rc::new(move |cx: &App| view.read(cx).has_unsaved()))
                },
                title: Rc::new(move |cx: &App| title_view.read(cx).title()),
            }
        });
    let diff: crate::shared::pane::PaneFactory =
        Rc::new(|path: PathBuf, window: &mut Window, cx: &mut App| {
            let root = repo_root(&path);
            let view = cx.new(|cx| FileView::diff(root, path, window, cx));
            let focus = view.focus_handle(cx);
            let title_view = view.clone();
            PaneItem {
                kind: PaneKind::File,
                view: view.into(),
                focus,
                title: Rc::new(move |cx: &App| title_view.read(cx).title()),
                unsaved: None,
            }
        });
    let history: crate::shared::pane::PaneFactory =
        Rc::new(|cwd: PathBuf, _: &mut Window, cx: &mut App| {
            let view = cx.new(|cx| HistoryView::new(cwd, cx));
            let focus = view.focus_handle(cx);
            let title_view = view.clone();
            PaneItem {
                kind: PaneKind::History,
                view: view.into(),
                focus,
                title: Rc::new(move |cx: &App| title_view.read(cx).title()),
                unsaved: None,
            }
        });
    cx.set_global(PaneFactories {
        terminal: Some(terminal),
        file: Some(file),
        diff: Some(diff),
        history: Some(history),
    });
}

/// Files | Changes | Branches follow the selected agent's folder. Views for
/// recently visited folders stay cached so their tab, expansion and scroll
/// position survive switching agents; hidden views never poll Git.
pub fn attach_sidebar(
    workspace: &Entity<crate::features::workspace::Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    use std::collections::{HashMap, VecDeque};

    use crate::features::files::{FileTreeEvent, FileTreeView};
    use crate::features::workspace::WorkspaceEvent;

    const CACHE_LIMIT: usize = 8;

    fn create_tree(
        workspace: &Entity<crate::features::workspace::Workspace>,
        root: PathBuf,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<FileTreeView> {
        let tree = cx.new(|cx| FileTreeView::new(root, window, cx));
        let target = workspace.downgrade();
        window
            .subscribe(&tree, cx, move |_, event: &FileTreeEvent, window, cx| {
                let _ = target.update(cx, |workspace, cx| match event {
                    FileTreeEvent::Open(path) => workspace.open_file(path.clone(), window, cx),
                    FileTreeEvent::OpenToSide(path) => {
                        workspace.open_file_beside(path.clone(), window, cx)
                    }
                    FileTreeEvent::OpenInNewTab(path) => {
                        workspace.open_file_in_tab(path.clone(), window, cx)
                    }
                    FileTreeEvent::OpenDiff(path) => workspace.open_diff(path.clone(), window, cx),
                    FileTreeEvent::OpenAgent(folder) => {
                        workspace.open_agent_tab(folder.clone(), window, cx)
                    }
                    FileTreeEvent::OpenHistory(repo) => {
                        workspace.open_history(repo.clone(), window, cx)
                    }
                    FileTreeEvent::InsertPath(text) => {
                        if !workspace.insert_into_composer(text, window, cx) {
                            window.push_notification(
                                Notification::info(
                                    "Open an agent to insert paths into its prompt.",
                                ),
                                cx,
                            );
                        }
                    }
                    FileTreeEvent::Collapse => workspace.toggle_sidebar(cx),
                });
            })
            .detach();
        tree
    }

    let root = workspace.read(cx).files_root(cx);
    let initial = create_tree(workspace, root.clone(), window, cx);
    let visible = workspace.read(cx).files_sidebar_visible();
    initial.update(cx, |tree, cx| tree.set_visible(visible, cx));
    workspace.update(cx, |workspace, cx| workspace.set_sidebar(initial.clone().into(), cx));

    let mut active = root.clone();
    let mut cache = HashMap::from([(root.clone(), initial)]);
    let mut recent = VecDeque::from([root]);

    // This single subscription controls the active tree. Inactive cached
    // trees remain invisible, so the Git pollers are suspended.
    window
        .subscribe(
            workspace,
            cx,
            move |owner, event: &WorkspaceEvent, window, cx| match event {
                WorkspaceEvent::FilesRootChanged(root) => {
                    if *root != active {
                        if let Some(old) = cache.get(&active) {
                            old.update(cx, |tree, cx| tree.set_visible(false, cx));
                        }
                        let tree = if let Some(cached) = cache.get(root) {
                            cached.clone()
                        } else {
                            let created = create_tree(&owner, root.clone(), window, cx);
                            cache.insert(root.clone(), created.clone());
                            created
                        };
                        active = root.clone();
                        recent.retain(|path| path != root);
                        recent.push_back(root.clone());
                        while cache.len() > CACHE_LIMIT {
                            let Some(oldest) = recent.pop_front() else {
                                break;
                            };
                            if oldest != active {
                                cache.remove(&oldest);
                            }
                        }
                        let visible = owner.read(cx).files_sidebar_visible();
                        tree.update(cx, |tree, cx| tree.set_visible(visible, cx));
                        owner.update(cx, |workspace, cx| workspace.set_sidebar(tree.into(), cx));
                    } else if let Some(tree) = cache.get(&active) {
                        let visible = owner.read(cx).files_sidebar_visible();
                        tree.update(cx, |tree, cx| tree.set_visible(visible, cx));
                    }
                }
                WorkspaceEvent::SidebarToggled(shown)
                | WorkspaceEvent::FilesVisibilityChanged(shown) => {
                    if let Some(tree) = cache.get(&active) {
                        let visible = *shown && owner.read(cx).files_sidebar_visible();
                        tree.update(cx, |tree, cx| tree.set_visible(visible, cx));
                    }
                }
            },
        )
        .detach();
}
