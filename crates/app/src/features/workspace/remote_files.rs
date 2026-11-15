//! Switch the right-hand file dock with the selected agent's machine.
//! Local FileTreeView is owned by the app integration. Remote browsing and
//! revision-checked editing use the selected Herdr agent's own RPC channel.

use super::*;

/// Scope a browser click to the currently selected remote agent's Linux
/// project; the bridge additionally checks symlinks and realpath.
pub(super) fn remote_file_in_root(root: &str, path: &str) -> bool {
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
    /// Open the common Commit History graph with the current Linux agent as
    /// its source. No Windows Git lookup or extra Pi session is created.
    pub(super) fn open_remote_history(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((session, machine, root)) = self.selected_remote_files(cx) else {
            window.push_notification(Notification::info(
                "Remote agent identity is not ready. Try again after connecting.",
            ), cx);
            return;
        };
        if root.is_empty() || root == "/" {
            window.push_notification(Notification::info(
                "Open the remote agent inside a Git project to view its history.",
            ), cx);
            return;
        }
        let pane = self.build_remote_history_pane(
            session, machine, root, cx,
        );
        self.place_preview(pane, window, cx);
    }

    /// Construct the same Git History pane during live opening or recovery.
    /// Its session reference reconnects to the original Herdr address.
    pub(super) fn build_remote_history_pane(
        &mut self, session: Entity<SessionView>,
        machine: String, root: String, cx: &mut Context<Self>,
    ) -> PaneId {
        let view = cx.new(|cx| HistoryView::new_remote(
            machine.clone(), root.clone(), session.clone(), cx,
        ));
        let title = view.clone();
        let item = PaneItem {
            kind: PaneKind::History,
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
            agent_target: Some(pi_rpc::AgentTarget::Remote {
                machine, cwd: root,
            }),
            title: SharedString::default(),
            running: false,
            needs_you: false,
            _subscriptions: Vec::new(),
        };
        pane.sync_chrome(cx);
        self.panes.insert(id, pane);
        id
    }

    /// Select the remote Files dock's Changes page for a command owned by
    /// the current agent, preserving the two independent sidebars.
    pub(super) fn show_selected_remote_changes(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.files_sidebar_open {
            self.toggle_files_panel(cx);
        }
        self.sync_remote_files(window, cx);
        let Some((session, _, root)) = self.selected_remote_files(cx) else {
            window.push_notification(Notification::info(
                "The selected remote agent has no known project directory.",
            ), cx);
            return;
        };
        if let Some(view) = &self.remote_files
            && view.read(cx).matches(&session, &root)
        {
            view.update(cx, |view, cx| view.show_changes(cx));
        } else {
            window.push_notification(Notification::info(
                "Remote Files is waiting for the selected agent.",
            ), cx);
        }
    }

    /// The native Architecture Docs installer expects local Windows files.
    /// Remote agents instead open the selected Linux docs directory through
    /// the existing Herdr file browser; never read/write Windows docs.
    pub(super) fn show_selected_remote_docs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.files_sidebar_open {
            self.toggle_files_panel(cx);
        }
        self.sync_remote_files(window, cx);
        let Some((session, _, root)) = self.selected_remote_files(cx) else {
            window.push_notification(Notification::info(
                "Remote project directory unavailable. Reconnect the agent.",
            ), cx);
            return;
        };
        if root.is_empty() || root == "/" {
            window.push_notification(Notification::info(
                "Select a Linux project directory before browsing architecture docs.",
            ), cx);
            return;
        }
        if let Some(view) = &self.remote_files
            && view.read(cx).matches(&session, &root)
        {
            view.update(cx, |view, cx| view.show_remote_docs(cx));
        } else {
            window.push_notification(Notification::info(
                "Remote Files is waiting for the selected agent.",
            ), cx);
        }
    }

    /// A remote branch result is tagged with the original machine and root.
    /// If the focus changes before the user picks it, fail closed instead of
    /// executing Git on a different Linux host or on Windows.
    pub(super) fn select_palette_remote_branch(
        &mut self,
        machine: &str,
        root: &str,
        terminal: &str,
        branch: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((session, active_machine, active_root)) = self.selected_remote_files(cx)
        else {
            window.push_notification(Notification::info(
                "Remote branch selection requires the original agent.",
            ), cx);
            return;
        };
        if terminal.is_empty()
            || active_machine != machine || active_root != root
            || session.read(cx).herdr_terminal().as_deref() != Some(terminal)
        {
            window.push_notification(Notification::info(
                "The selected remote agent changed. Reopen the palette.",
            ), cx);
            return;
        }
        if !self.files_sidebar_open {
            self.toggle_files_panel(cx);
        }
        self.sync_remote_files(window, cx);
        if let Some(view) = &self.remote_files
            && view.read(cx).matches(&session, &active_root)
        {
            view.update(cx, |view, cx| view.select_remote_branch(branch, cx));
        }
    }

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
        let pane = self.build_remote_file_pane(
            session, machine, root, path, window, cx,
        );
        self.place_preview(pane, window, cx);
    }

    /// Instantiate a remote editor owned by one original agent. Restoration
    /// and normal opening share exactly this constructor, including the
    /// observer that schedules draft snapshots on every editor change.
    pub(super) fn build_remote_file_pane(
        &mut self,
        session: Entity<SessionView>,
        machine: String,
        root: String,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneId {
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
            terminal_address: None,
        };
        let subscription = cx.observe(&file, |this, _, cx| {
            this.schedule_save(cx);
        });
        let id = self.alloc_id();
        let mut pane = Pane {
            item,
            session: None,
            linked_session: Some(session),
            cwd: self.space().root.clone(),
            path: None, // Linux paths never become Windows PathBuf values.
            session_file: None,
            herdr_address: None,
            agent_target: Some(pi_rpc::AgentTarget::Remote { machine, cwd: root }),
            title: SharedString::default(),
            running: false,
            needs_you: false,
            _subscriptions: vec![subscription],
        };
        pane.sync_chrome(cx);
        self.panes.insert(id, pane);
        id
    }

    /// Relay a file directly through two existing Herdr RPC processes.
    /// Source and destination addresses are exact, not selected machine names.
    pub(super) fn relay_remote_file(
        &mut self,
        source: Entity<SessionView>,
        path: String,
        destination_terminal: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((active, _, root)) = self.selected_remote_files(cx) else {
            return;
        };
        if active != source || !remote_file_in_root(&root, &path) {
            window.push_notification(Notification::info(
                "The source agent changed. Select its remote file again.",
            ), cx);
            return;
        }
        let Some(source_process) = source.read(cx).remote_fs_process() else {
            window.push_notification(Notification::info(
                "The source remote agent is disconnected.",
            ), cx);
            return;
        };
        let target = self.sessions().into_iter().find(|candidate| {
            candidate != &source && candidate.read(cx).is_remote_herdr()
                && candidate.read(cx).herdr_terminal().as_deref()
                    == Some(destination_terminal.as_str())
        });
        let Some(target) = target else {
            window.push_notification(Notification::info(
                "The destination agent is no longer connected.",
            ), cx);
            return;
        };
        let (Some((_, destination_root)), Some(destination_process)) = (
            target.read(cx).remote_fs_target(),
            target.read(cx).remote_fs_process(),
        ) else {
            window.push_notification(Notification::info(
                "Destination agent has no connected project directory.",
            ), cx);
            return;
        };
        let transfer = cx.background_executor().spawn(
            crate::features::files::transfers::relay(
                root, source_process, path,
                destination_root.clone(), destination_process, destination_root,
            ),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = transfer.await;
            let _ = this.update_in(cx, |_, window, cx| {
                match result {
                    Ok(path) => window.push_notification(Notification::success(
                        format!("Copied to destination agent: {path}"),
                    ), cx),
                    Err(error) => window.push_notification(Notification::error(
                        format!("Remote transfer failed: {error}"),
                    ), cx),
                }
            });
        }).detach();
    }

    /// Remote filesystem root is the agent's launch directory, never the
    /// locally saved Windows workspace root.
    pub(super) fn selected_remote_files(&self, cx: &App) -> Option<(Entity<SessionView>, String, String)> {
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
        // Copy targets are other live remote agents, identified by their
        // exact Herdr terminal, never a stale machine label.
        let targets: Vec<(String, String)> = self.sessions().into_iter()
            .filter_map(|candidate| {
                if selected.as_ref().is_some_and(|(session, _, _)| session == &candidate) {
                    return None;
                }
                let agent = candidate.read(cx);
                let (machine, _) = agent.remote_fs_target()?;
                agent.remote_fs_process()?;
                let terminal = agent.herdr_terminal()?;
                Some((format!("{machine} ({terminal})"), terminal))
            })
            .take(12)
            .collect();
        if let Some((session, _, root)) = &selected
            && let Some(view) = self.remote_files.as_ref().filter(|view| {
                view.read(cx).matches(session, root)
            })
        {
            view.update(cx, |view, cx| view.set_transfer_targets(targets, cx));
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
        view.update(cx, |view, cx| view.set_transfer_targets(targets, cx));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, source, event: &RemoteFilesEvent, window, cx| {
                match event {
                    RemoteFilesEvent::Close => this.toggle_files_panel(cx),
                    RemoteFilesEvent::RelayFile { path, target_terminal } => {
                        let origin = source.read(cx).session().clone();
                        this.relay_remote_file(
                            origin, path.clone(), target_terminal.clone(), window, cx,
                        );
                    },
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
