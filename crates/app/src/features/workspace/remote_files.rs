//! Switch the right-hand file dock with the selected agent's machine.
//! Local FileTreeView is owned by the app integration; remote browsing is
//! strictly read-only and uses the selected Herdr agent's own RPC channel.

use super::*;

impl Workspace {
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
        let Some((session, machine, root)) = selected else {
            self.remote_files = None;
            self._remote_files_events = None;
            return;
        };
        if self.remote_files.as_ref().is_some_and(|view| {
            view.read(cx).matches(&session, &root)
        }) {
            return;
        }
        let view = cx.new(|cx| RemoteFilesView::new(session, machine, root, cx));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, source, event: &RemoteFilesEvent, window, cx| {
                match event {
                    RemoteFilesEvent::Close => this.toggle_files_panel(cx),
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
