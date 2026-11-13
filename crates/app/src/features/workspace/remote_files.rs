//! Switch the right-hand file dock with the selected agent's machine.
//! Local FileTreeView is owned by the app integration. Remote browsing and
//! revision-checked editing use the selected Herdr agent's own RPC channel.

use super::*;

/// Scope a browser click to the currently selected remote agent's Linux
/// project; the bridge additionally checks symlinks and realpath.
fn remote_file_in_root(root: &str, path: &str) -> bool {
    if root.is_empty() || !root.starts_with('/') || !path.starts_with('/')
        || path == root || path.len() > 2048 || path.contains('\\')
        || path.bytes().any(|byte| byte < 32 || byte == 127)
        || path.split('/').any(|part| part == "." || part == "..")
    {
        return false;
    }
    root == "/" || path.starts_with(&(root.to_owned() + "/"))
}

impl Workspace {
    /// Render the selected agent's remote diff in a central native Diff pane.
    /// It borrows, but never owns or terminates, the underlying Pi agent.
    pub(super) fn open_remote_git_diff(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((session, machine, root)) = self.selected_remote_files(cx) else {
            return;
        };
        if !remote_file_in_root(&root, &path) {
            window.push_notification(Notification::info(
                "Remote Git diff path is outside the selected project.",
            ), cx);
            return;
        }
        let view = cx.new(|cx| RemoteGitDiffView::new(
            session.clone(), machine.clone(), root.clone(), path, cx,
        ));
        let title = view.clone();
        let item = PaneItem {
            kind: PaneKind::File,
            view: view.clone().into(),
            focus: view.focus_handle(cx),
            title: Rc::new(move |cx: &App| title.read(cx).title()),
            unsaved: None,
            terminal_address: None,
        };
        let id = self.alloc_id();
        let mut pane = Pane {
            item,
            session: None,
            linked_session: Some(session),
            cwd: self.space().root.clone(),
            path: None,
            session_file: None,
            herdr_address: None,
            agent_target: Some(pi_rpc::AgentTarget::Remote { machine, cwd: root }),
            title: SharedString::default(),
            running: false,
            needs_you: false,
            _subscriptions: Vec::new(),
        };
        pane.sync_chrome(cx);
        self.panes.insert(id, pane);
        self.place_preview(id, window, cx);
    }

    /// A selected Linux file opens beside its agent in the main workspace,
    /// just like a local FileView preview. Files stays a directory browser.
    pub(super) fn open_remote_file(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((session, machine, root)) = self.selected_remote_files(cx) else {
            return;
        };
        // A path is never translated to Windows. This preliminary lexical
        // check protects the UI; Herdr bridge also validates realpath/root.
        if !remote_file_in_root(&root, &path) {
            window.push_notification(Notification::info(
                "Remote path is outside the selected agent's project.",
            ), cx);
            return;
        }
        let file = cx.new(|cx| RemoteFilesView::new_pane(
            session.clone(), machine.clone(), root.clone(), path, window, cx,
        ));
        let title_view = file.clone();
        let unsaved_view = file.clone();
        let item = PaneItem {
            kind: PaneKind::File,
            view: file.clone().into(),
            focus: file.focus_handle(cx),
            title: Rc::new(move |cx: &App| title_view.read(cx).file_title(cx)),
            unsaved: Some(Rc::new(move |cx: &App| {
                unsaved_view.read(cx).has_unsaved(cx)
            })),
            // File panes never own or shut down the connected Herdr agent.
            terminal_address: None,
        };
        let id = self.alloc_id();
        let mut pane = Pane {
            item,
            session: None,
            linked_session: Some(session),
            cwd: self.space().root.clone(),
            path: None, // Not a Windows path; never restore as a local file.
            session_file: None,
            herdr_address: None,
            agent_target: Some(pi_rpc::AgentTarget::Remote { machine, cwd: root }),
            title: SharedString::default(),
            running: false,
            needs_you: false,
            _subscriptions: Vec::new(),
        };
        pane.sync_chrome(cx);
        self.panes.insert(id, pane);
        self.place_preview(id, window, cx);
    }

    /// Remote filesystem root is the agent's launch directory, never the
    /// locally saved Windows workspace root.
    fn selected_remote_files(&self, cx: &App) -> Option<(Entity<SessionView>, String, String)> {
        let session = self.active_session()?;
        let agent = session.read(cx);
        if !agent.is_remote_herdr() {
            return None;
        }
        if let Some((machine, root)) = agent.remote_fs_target() {
            return Some((session.clone(), machine, root));
        }
        let address = agent.herdr_terminal()?;
        let (machine, _) = address.split_once('/')?;
        let root = self.machine_snapshot.as_ref()
            .and_then(|snapshot| snapshot.terminals.iter()
                .find(|terminal| terminal.address == address))
            .map_or_else(String::new, |terminal| terminal.cwd.clone());
        Some((session.clone(), machine.to_owned(), root))
    }

    /// Ensure a single view is bound to the *currently selected* remote
    /// session. A late result from a previously selected agent cannot appear
    /// in the new agent's right panel or insert a path in the wrong prompt.
    pub(super) fn sync_remote_files(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self.selected_remote_files(cx);
        if let Some((session, _, root)) = &selected
            && self.remote_files.as_ref().is_some_and(|view| {
                view.read(cx).matches(session, root)
            })
        {
            return;
        }
        // Retain unsaved editors, including an in-flight save. Dropping an
        // Entity on machine switch would otherwise lose the user's draft.
        if let Some(previous) = self.remote_files.take()
            && previous.read(cx).has_draft()
        {
            self.remote_file_drafts.push(previous);
        }
        self._remote_files_events = None;
        // A saved draft may finish while another agent is selected.
        self.remote_file_drafts.retain(|view| view.read(cx).has_draft());
        let Some((session, machine, root)) = selected else {
            return;
        };
        let view = if let Some(index) = self.remote_file_drafts.iter()
            .position(|view| view.read(cx).matches(&session, &root))
        {
            self.remote_file_drafts.remove(index)
        } else {
            cx.new(|cx| RemoteFilesView::new(session, machine, root, window, cx))
        };
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, source, event: &RemoteFilesEvent, window, cx| {
                match event {
                    RemoteFilesEvent::Close => this.toggle_files_panel(cx),
                    RemoteFilesEvent::OpenDiff(path) => {
                        if this.active_session().as_ref()
                            != Some(source.read(cx).session())
                        {
                            return;
                        }
                        this.open_remote_git_diff(path.clone(), window, cx);
                    },
                    RemoteFilesEvent::OpenFile(path) => {
                        if this.active_session().as_ref()
                            != Some(source.read(cx).session())
                        {
                            return;
                        }
                        this.open_remote_file(path.clone(), window, cx);
                    },
                    RemoteFilesEvent::InsertPath(path) => {
                        if this.active_session().as_ref()
                            != Some(source.read(cx).session())
                        {
                            return;
                        }
                        if !this.insert_into_composer(path, window, cx) {
                            window.push_notification(Notification::info(
                                "The remote agent is not ready to receive this path.",
                            ), cx);
                        }
                    }
                }
            },
        );
        self.remote_files = Some(view);
        self._remote_files_events = Some(sub);
    }
}

#[cfg(test)]
mod tests {
    use super::remote_file_in_root;

    #[test]
    fn remote_file_navigation_is_machine_and_root_scoped() {
        assert!(remote_file_in_root("/srv/workspaces/JaviPi",
            "/srv/workspaces/JaviPi/prepare-boot.ps1"));
        assert!(!remote_file_in_root("/srv/workspaces/JaviPi",
            "/srv/workspaces/JaviPi2/prepare-boot.ps1"));
        assert!(!remote_file_in_root("/srv/workspaces/JaviPi",
            "/srv/workspaces/Other/demo.txt"));
        assert!(!remote_file_in_root("/srv/workspaces/JaviPi",
            "/srv/workspaces/JaviPi/../Other/demo.txt"));
        assert!(!remote_file_in_root("/srv/workspaces/JaviPi",
            "C:\\Users\\demo\\readme.md"));
        assert!(!remote_file_in_root("/", "/"));
        assert!(remote_file_in_root("/", "/etc/hosts"));
        assert!(!remote_file_in_root("/", "/etc/../hosts"));
    }
}
