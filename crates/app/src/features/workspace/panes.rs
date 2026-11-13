//! Pane creation, preview placement, focus, splits, closing and swapping.

use super::*;

/// Construction-only settings for an agent pane. Grouping them avoids a
/// fragile positional argument list and keeps old/restored/local targets
/// explicit without changing any runtime behavior.
#[derive(Default)]
struct AgentPaneInit {
    prompt: Option<String>,
    resume: Option<PathBuf>,
    restored_herdr: Option<String>,
    agent_target: Option<pi_rpc::AgentTarget>,
    start_now: bool,
}

impl Workspace {
    /// Puts a freshly built file/diff pane in the current tab: it replaces the
    /// tab's existing preview pane, else splits the largest pane along its
    /// long side, else (no room for two panes) gets its own tab.
    pub(super) fn place_preview(
        &mut self,
        new: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tab() else {
            self.push_pane_tab(new, window, cx);
            return;
        };
        let preview = tab.tree.leaves().into_iter().find(|id| {
            *id != new
                && self
                    .panes
                    .get(id)
                    .is_some_and(|p| p.item.kind == PaneKind::File
                        && !p.item.unsaved.as_ref().is_some_and(|probe| probe(cx)))
        });
        if let Some(old) = preview
            && let Some(pane) = self.panes.remove(&new)
        {
            self.panes.insert(old, pane);
            self.focus_pane(old, window, cx);
            self.schedule_save(cx);
            return;
        }
        let largest = self
            .pane_sizes(window)
            .into_iter()
            .max_by(|a, b| (a.1 * a.2).total_cmp(&(b.1 * b.2)));
        let Some((target, w, h)) = largest else {
            self.push_pane_tab(new, window, cx);
            return;
        };
        let axis = if w >= h * 1.2 {
            SplitAxis::Horizontal
        } else {
            SplitAxis::Vertical
        };
        if !fits_split(axis, w, h) {
            self.push_pane_tab(new, window, cx);
            return;
        }
        if let Some(tab) = self.tab_mut() {
            tab.tree.split(&target, new, axis);
        }
        self.focus_pane(new, window, cx);
        self.schedule_save(cx);
    }

    /// Opens an already built pane as a new tab.
    pub(super) fn push_pane_tab(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.alloc_id();
        let space = self.space_mut();
        space.tabs.push(Tab {
            id,
            tree: PaneTree::Leaf(pane),
            focused: pane,
        });
        let index = space.tabs.len() - 1;
        self.activate(index, window, cx);
    }

    /// Approximate on-screen size of each pane in the current tab.
    pub(super) fn pane_sizes(&self, window: &Window) -> Vec<(PaneId, f32, f32)> {
        let viewport = window.viewport_size();
        let sidebar = if self.sidebar.is_some() && self.sidebar_visible {
            SIDEBAR_WIDTH + 6.
        } else {
            0.
        };
        let w = f32::from(viewport.width) - 12. - sidebar;
        let h = f32::from(viewport.height) - 36. - 6.;
        let mut out = Vec::new();
        if let Some(tab) = self.tab() {
            leaf_sizes(&tab.tree, w, h, &mut out);
        }
        out
    }

    // ── Workspaces ──────────────────────────────────────────────────────────

    pub(super) fn pane_cwd(&self, pane: &Pane, cx: &App) -> PathBuf {
        match &pane.session {
            Some(session) => session.read(cx).cwd().clone(),
            None => pane.cwd.clone(),
        }
    }

    /// An agent pane; its backend starts now with `start_now` (the pane is in
    /// the tab being shown), else the first time its tab is shown.
    fn new_agent_pane(
        &mut self,
        cwd: PathBuf,
        options: AgentPaneInit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneId {
        let AgentPaneInit {
            prompt,
            resume,
            restored_herdr,
            agent_target,
            start_now,
        } = options;
        let session = cx.new(|cx| {
            let mut session = SessionView::new(cwd.clone(), prompt, window, cx, false);
            if let Some(address) = restored_herdr.clone() {
                session.restore_herdr_terminal(address);
            }
            session.set_agent_target(agent_target.clone());
            session
        });
        if start_now {
            session.update(cx, |s, cx| s.ensure_started(window, cx));
        }
        if let Some(path) = resume.clone() {
            session.update(cx, |s, cx| s.resume_when_ready(path, cx));
        }
        let id = self.alloc_id();
        let focus = session.read(cx).composer().read(cx).focus_handle(cx);
        let title = {
            let session = session.clone();
            Rc::new(move |cx: &App| SharedString::from(session.read(cx).title()))
        };
        let subscriptions = vec![cx.subscribe_in(
            &session,
            window,
            move |this, session, event: &SessionEvent, window, cx| match event {
                // Tab titles, running dots and the saved conversation follow
                // each session; its other changes don't touch the chrome.
                SessionEvent::ChromeChanged => {
                    let file = session.read(cx).session_file();
                    let Some(pane) = this.panes.get_mut(&id) else {
                        return;
                    };
                    let changed = pane.sync_chrome(cx);
                    if pane.session_file != file {
                        pane.session_file = file;
                        this.schedule_save(cx);
                    }
                    if changed {
                        // An open switcher lists every workspace's activity.
                        if let Some((Overlay::Palette(view), _)) = &this.overlay {
                            let (view, rows) = (view.clone(), this.workspace_rows());
                            view.update(cx, |view, cx| view.set_workspaces(rows, cx));
                        }
                        cx.notify();
                    }
                }
                SessionEvent::OpenProviders => this.open_providers(window, cx),
                SessionEvent::OpenRemoteDirectory(machine) => {
                    this.open_remote_directory_picker(machine.clone(), window, cx)
                },
                SessionEvent::OpenScreen(screen) => {
                    this.open_screen(*screen, Some(session.clone()), window, cx)
                }
                SessionEvent::Effort(level) => {
                    if let Some(level) = crate::shared::surge::Level::of(level) {
                        this.play_surge(level, cx)
                    }
                }
                SessionEvent::DesignCanvas(canvas) => {
                    this.show_design_canvas(canvas.clone(), id, window, cx)
                }
                SessionEvent::Device(intent) => {
                    // A device an agent starts itself still shows in the app.
                    if crate::features::devices::hub(cx).read(cx).prefs().enabled {
                        let intent = intent.clone();
                        this.devices
                            .update(cx, |panel, cx| panel.follow(intent, cx));
                    }
                }
                SessionEvent::DeviceCommand(command) => {
                    let command = command.clone();
                    this.devices
                        .update(cx, |panel, cx| panel.activity(command, cx));
                }
                SessionEvent::OpenWeb(url) => {
                    this.open_web(url.clone(), Some(id), true, window, cx)
                }
                SessionEvent::AgentWeb(url) => {
                    this.open_web(url.clone(), Some(id), false, window, cx)
                }
                SessionEvent::Notice { level, message } => {
                    let source = this.pane_title(id);
                    let (level, message) = (*level, message.clone());
                    this.notices.update(cx, |store, cx| {
                        store.push(level, message, source, Some(id), std::time::Instant::now());
                        cx.notify();
                    });
                    cx.notify();
                }
            },
        )];
        let mut pane = Pane {
            item: PaneItem {
                kind: PaneKind::Agent,
                view: session.clone().into(),
                focus,
                title,
                unsaved: None,
                terminal_address: Some({
                    let session = session.clone();
                    Rc::new(move |cx: &App| session.read(cx).herdr_terminal())
                }),
            },
            session: Some(session),
            linked_session: None,
            cwd,
            path: None,
            session_file: resume,
            herdr_address: restored_herdr,
            agent_target,
            title: SharedString::default(),
            running: false,
            needs_you: false,
            _subscriptions: subscriptions,
        };
        pane.sync_chrome(cx);
        self.panes.insert(id, pane);
        id
    }

    /// A terminal (`target` = folder) or file (`target` = file) pane from the
    /// registered factory; a notice when there is none.
    pub(super) fn new_factory_pane(
        &mut self,
        kind: PaneKind,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        self.new_factory_pane_restored(kind, target, None, window, cx)
    }

    fn new_factory_pane_restored(
        &mut self,
        kind: PaneKind,
        target: PathBuf,
        herdr_terminal: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let factories = PaneFactories::get(cx);
        let available = match kind {
            PaneKind::Terminal => factories.terminal.is_some(),
            PaneKind::File => factories.file.is_some(),
            PaneKind::Agent => false,
        };
        if !available {
            window.push_notification(
                Notification::info(format!(
                    "{} panes are not available in this build.",
                    match kind {
                        PaneKind::File => "File",
                        _ => "Terminal",
                    }
                )),
                cx,
            );
            return None;
        };
        // Retain persisted Herdr identity across a native/local-only run.
        // The terminal view will use a native PTY and never call hc in that mode.
        let dormant_herdr_address = if crate::features::terminal::herdr_enabled() {
            None
        } else {
            herdr_terminal.clone()
        };
        let item = match kind {
            PaneKind::Terminal => (factories.terminal?)(target.clone(), herdr_terminal, window, cx),
            PaneKind::File => (factories.file?)(target.clone(), window, cx),
            PaneKind::Agent => return None,
        };
        let id = self.alloc_id();
        let (cwd, path) = match kind {
            PaneKind::File => (
                target
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.space().root.clone()),
                Some(target),
            ),
            _ => (target, None),
        };
        let mut pane = Pane {
            item,
            session: None,
            linked_session: None,
            cwd,
            path,
            session_file: None,
            herdr_address: dormant_herdr_address,
            agent_target: None,
            title: SharedString::default(),
            running: false,
            needs_you: false,
            _subscriptions: Vec::new(),
        };
        pane.sync_chrome(cx);
        self.panes.insert(id, pane);
        Some(id)
    }

    pub(super) fn new_pane(
        &mut self,
        kind: PaneKind,
        target: PathBuf,
        prompt: Option<String>,
        resume: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        match kind {
            PaneKind::Agent => Some(self.new_agent_pane(
                target,
                AgentPaneInit { prompt, resume, start_now: true, ..Default::default() },
                window,
                cx,
            )),
            PaneKind::Terminal | PaneKind::File => self.new_factory_pane(kind, target, window, cx),
        }
    }

    pub(super) fn restore_pane(
        &mut self,
        saved: &SavedPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let cwd = if saved.cwd.is_dir() {
            saved.cwd.clone()
        } else if self.space().root.is_dir() {
            self.space().root.clone()
        } else {
            crate::cli::default_cwd()
        };
        match saved.kind {
            PaneKind::Agent => {
                let resume = saved.session.clone().filter(|p| p.exists());
                // Started once its tab is shown (see `start_shown`).
                Some(self.new_agent_pane(
                    cwd,
                    AgentPaneInit {
                        resume,
                        restored_herdr: saved.herdr_terminal.clone(),
                        agent_target: saved.agent_target.clone(),
                        ..Default::default()
                    },
                    window,
                    cx,
                ))
            }
            PaneKind::Terminal => {
                PaneFactories::get(cx).terminal.as_ref()?;
                self.new_factory_pane_restored(PaneKind::Terminal, cwd, saved.herdr_terminal.clone(), window, cx)
            }
            PaneKind::File => {
                let path = saved.path.clone().filter(|p| p.is_file())?;
                PaneFactories::get(cx).file.as_ref()?;
                self.new_factory_pane(PaneKind::File, path, window, cx)
            }
        }
    }

    /// Attach an existing Herdr address as a new GPUI tab without running a
    /// replacement shell. Only available when Herdr is the selected backend.
    pub(super) fn open_herdr_terminal(
        &mut self,
        address: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !crate::features::terminal::herdr_enabled() {
            return;
        }
        let cwd = self.active_cwd(cx);
        if let Some(pane) = self.new_factory_pane_restored(
            PaneKind::Terminal, cwd, Some(address), window, cx,
        ) {
            self.push_pane_tab(pane, window, cx);
        }
    }

    /// One-time recovery of agents started by older 4B builds, before agent
    /// pane addresses were saved. This only opens new views of known live
    /// Herdr RPC terminals; no terminal.run or second Pi agent is issued.
    pub(super) fn recover_untracked_agents(
        &mut self,
        addresses: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut known: std::collections::HashSet<String> = self.panes.values()
            .filter(|pane| pane.item.kind == PaneKind::Agent)
            .filter_map(|pane| {
                pane.herdr_address.clone().or_else(|| {
                    pane.item.terminal_address.as_ref().and_then(|get| get(cx))
                })
            })
            .collect();
        // Other workspaces may not have been instantiated yet; keep their
        // saved identities reserved to prevent duplicate recovered tabs.
        for space in &self.spaces {
            if let Some((tabs, _)) = &space.saved {
                for tab in tabs {
                    for pane in tab.layout.leaves() {
                        if pane.kind == PaneKind::Agent
                            && let Some(address) = pane.herdr_terminal
                        {
                            known.insert(address);
                        }
                    }
                }
            }
        }

        let mut first_recovered = None;
        for address in addresses {
            if !known.insert(address.clone()) {
                continue;
            }
            let cwd = self.space().root.clone();
            let pane = self.new_agent_pane(
                cwd,
                AgentPaneInit { restored_herdr: Some(address), ..Default::default() },
                window,
                cx,
            );
            let id = self.alloc_id();
            let space = self.space_mut();
            if first_recovered.is_none() {
                first_recovered = Some(space.tabs.len());
            }
            space.tabs.push(Tab {
                id,
                tree: PaneTree::Leaf(pane),
                focused: pane,
            });
        }
        if let Some(index) = first_recovered {
            // After upgrading from 4B's address-less layout, show a recovered
            // agent rather than starting an unwanted replacement Pi process.
            self.space_mut().active = index;
            self.focus_active(window, cx);
            self.schedule_save(cx);
            cx.notify();
        }
    }

    /// If all live agents were already present in saved workspaces, the
    /// recovery pass adds no tabs. Do not leave a legacy address-less pane
    /// active: focus an existing live agent instead.
    pub(super) fn focus_existing_remote_agent(
        &mut self,
        addresses: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let live: std::collections::HashSet<&str> =
            addresses.iter().map(String::as_str).collect();
        let existing_tab = self.space().tabs.iter().position(|tab| {
            tab.tree.leaves().iter().any(|id| {
                self.panes
                    .get(id)
                    .and_then(|pane| pane.herdr_address.as_deref())
                    .is_some_and(|address| live.contains(address))
            })
        });
        if let Some(index) = existing_tab
            && self.space().active != index
        {
            self.space_mut().active = index;
            self.focus_active(window, cx);
            self.schedule_save(cx);
            cx.notify();
        }
    }

    /// Explicitly create a NEW agent under a single chosen Herdr machine.
    /// Unlike workspace restore, this is a deliberate terminal.run action.
    pub(super) fn open_agent_on_machine(
        &mut self,
        target: pi_rpc::AgentTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !crate::features::terminal::herdr_enabled() {
            return;
        }
        let cwd = self.active_cwd(cx);
        let pane = self.new_agent_pane(
            cwd,
            AgentPaneInit { agent_target: Some(target), start_now: true, ..Default::default() },
            window,
            cx,
        );
        self.push_pane_tab(pane, window, cx);
    }

    pub(super) fn open_tab(
        &mut self,
        kind: PaneKind,
        target: PathBuf,
        prompt: Option<String>,
        resume: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.new_pane(kind, target, prompt, resume, window, cx) else {
            return;
        };
        self.push_pane_tab(pane, window, cx);
    }

    pub(super) fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let space = self.space_mut();
        space.active = index.min(space.tabs.len().saturating_sub(1));
        self.focus_active(window, cx);
        self.start_shown(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Start visible restored agents only when it is safe. A live remote
    /// agent must be reattached by its address, not replaced by an old
    /// placeholder without a Herdr identity.
    pub(super) fn start_shown(&self, window: &mut Window, cx: &mut App) {
        let Some(tab) = self.tab() else { return };
        for id in tab.tree.leaves() {
            let Some(pane) = self.panes.get(&id) else { continue };
            if !self.remote_agent_recovery.may_auto_start(pane.herdr_address.is_some()) {
                continue;
            }
            if let Some(session) = &pane.session {
                session.update(cx, |s, cx| s.ensure_started(window, cx));
            }
        }
    }

    pub(super) fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(pane) = self.focused_pane().and_then(|id| self.panes.get(&id)) {
            pane.item.focus.focus(window, cx);
        }
    }

    pub(super) fn focus_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tab_mut()
            && tab.tree.contains(&pane)
        {
            tab.focused = pane;
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Show the pane wherever it lives: its workspace, its tab, then focus.
    pub(super) fn reveal_pane(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = self.spaces.iter().enumerate().find_map(|(s, space)| {
            space
                .tabs
                .iter()
                .position(|tab| tab.tree.contains(&pane))
                .map(|t| (s, t))
        });
        let Some((space, tab)) = found else {
            return;
        };
        if space != self.current {
            self.switch_workspace(space, window, cx);
        }
        self.activate(tab, window, cx);
        self.focus_pane(pane, window, cx);
    }

    /// Split the focused pane: with `what`, that kind and target; otherwise
    /// the focused pane's kind in its folder (or on its file).
    pub(super) fn split(
        &mut self,
        axis: SplitAxis,
        what: Option<(PaneKind, PathBuf)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(focused) = self.focused_pane() else {
            let root = self.space().root.clone();
            self.open_tab(PaneKind::Agent, root, None, None, window, cx);
            return;
        };
        let (kind, target) = match what {
            Some(what) => what,
            None => {
                let pane = &self.panes[&focused];
                let kind = pane.item.kind;
                let target = match (kind, &pane.path) {
                    (PaneKind::File, Some(path)) => path.clone(),
                    _ => self.pane_cwd(pane, cx),
                };
                (kind, target)
            }
        };
        let Some(pane) = self.new_pane(kind, target, None, None, window, cx) else {
            return;
        };
        let room = self
            .pane_sizes(window)
            .into_iter()
            .find(|(id, ..)| *id == focused)
            .is_none_or(|(_, w, h)| fits_split(axis, w, h));
        if !room {
            self.push_pane_tab(pane, window, cx);
            return;
        }
        if let Some(tab) = self.tab_mut() {
            tab.tree.split(&focused, pane, axis);
        }
        self.focus_pane(pane, window, cx);
        self.schedule_save(cx);
    }

    /// A new pane tiled like Hyprland's dwindle: the focused pane splits
    /// along its longer side (the other one if only that fits), so panes
    /// spiral in instead of lining up in one row. No room: a new tab.
    pub(super) fn tile(
        &mut self,
        what: Option<(PaneKind, PathBuf)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.focused_pane();
        let axis = self
            .pane_sizes(window)
            .into_iter()
            .find(|(id, ..)| Some(*id) == focused)
            .and_then(|(_, w, h)| tile_axis(w, h))
            // No size yet, or no room either way: `split` opens a tab.
            .unwrap_or(SplitAxis::Horizontal);
        self.split(axis, what, window, cx);
    }

    pub(super) fn split_with_kind(
        &mut self,
        kind: PaneKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.active_cwd(cx);
        self.tile(Some((kind, cwd)), window, cx);
    }

    /// Drop `moving` on `target` at `zone`: beside it on that side, or swap
    /// the two in the middle. A side whose halves wouldn't fit swaps instead.
    pub(super) fn move_pane(
        &mut self,
        moving: PaneId,
        target: PaneId,
        zone: DropZone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placement = zone.placement().filter(|(axis, _)| {
            self.pane_sizes(window)
                .into_iter()
                .find(|(id, ..)| *id == target)
                .is_none_or(|(_, w, h)| fits_split(*axis, w, h))
        });
        let Some((axis, before)) = placement else {
            self.swap_panes(moving, target, cx);
            return;
        };
        if moving == target {
            return;
        }
        if let Some(tab) = self.tab_mut() {
            let tree = std::mem::replace(&mut tab.tree, PaneTree::Leaf(moving));
            tab.tree = tree.moved(&moving, &target, axis, before);
            tab.focused = moving;
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// Close `pane`, asking first when it holds unsaved edits or its agent
    /// is working or waiting on you.
    pub(super) fn close_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let unsaved = self.unsaved_titles([pane], cx);
        let busy = self.busy_titles([pane]);
        if unsaved.is_empty() && busy.is_empty() {
            self.close_pane_now(pane, window, cx);
        } else {
            self.confirm_close(unsaved, busy, window, cx, move |this, window, cx| {
                this.close_pane_now(pane, window, cx)
            });
        }
    }

    /// Titles of the file panes among `panes` with edits not yet saved.
    pub(super) fn unsaved_titles(
        &self,
        panes: impl IntoIterator<Item = PaneId>,
        cx: &App,
    ) -> Vec<SharedString> {
        panes
            .into_iter()
            .filter_map(|id| self.panes.get(&id))
            .filter(|p| p.item.unsaved.as_ref().is_some_and(|u| u(cx)))
            .map(|p| (p.item.title)(cx))
            .collect()
    }

    /// Titles of the agents among `panes` in the middle of something:
    /// running a turn or waiting on a dialog. Closing one stops it.
    pub(super) fn busy_titles(&self, panes: impl IntoIterator<Item = PaneId>) -> Vec<SharedString> {
        panes
            .into_iter()
            .filter_map(|id| self.panes.get(&id))
            .filter(|p| p.running || p.needs_you)
            .map(|p| p.title.clone())
            .collect()
    }

    /// "Close anyway?" for files with unsaved edits and agents at work;
    /// `then` runs only on confirm.
    pub(super) fn confirm_close(
        &mut self,
        unsaved: Vec<SharedString>,
        busy: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let names = |titles: &[SharedString]| {
            titles
                .iter()
                .map(|t| format!("\u{201c}{}\u{201d}", t.trim_end_matches(" •")))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let title = match (unsaved.len(), busy.len()) {
            (0, 1) => format!("Stop {} and close it?", names(&busy)),
            (0, n) => format!("Stop {n} agents and close them?"),
            (1, 0) => format!("Close {} without saving?", names(&unsaved)),
            (n, 0) => format!("Close {n} files without saving?"),
            _ => "Close unsaved files and working agents?".to_string(),
        };
        let mut lines = Vec::new();
        if !unsaved.is_empty() {
            lines.push(format!(
                "Unsaved edits in {} are lost. ⌘S saves first.",
                names(&unsaved)
            ));
        }
        if !busy.is_empty() {
            lines.push(format!(
                "{} {} working or waiting for your answer; closing stops {}. The \
                 conversation is kept: {} reopens it.",
                names(&busy),
                if busy.len() == 1 { "is" } else { "are" },
                if busy.len() == 1 { "it" } else { "them" },
                closed::REOPEN_KEY,
            ));
        }
        let description = lines.join(" ");
        let ok = if busy.is_empty() {
            "Close without saving"
        } else {
            "Stop and close"
        };
        let this = cx.entity().downgrade();
        let then = Rc::new(then);
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (this, then) = (this.clone(), then.clone());
            alert
                .title(title.clone())
                .description(description.clone())
                .ok_text(ok)
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let then = then.clone();
                    let _ = this.update(cx, |this, cx| then(this, window, cx));
                    true
                })
        });
    }

    pub(super) fn close_pane_now(
        &mut self,
        pane: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .space()
            .tabs
            .iter()
            .position(|t| t.tree.contains(&pane))
        else {
            return;
        };
        if self.space().tabs[index].tree.len() == 1 {
            self.close_tab_now(index, window, cx);
            return;
        }
        self.remember_close(index, &[pane], cx);
        let active = index == self.space().active;
        let tab = &mut self.space_mut().tabs[index];
        let next = tab.tree.focus_after_close(&pane);
        if let Some(tree) = tab.tree.clone().without(&pane) {
            tab.tree = tree;
        }
        if tab.focused == pane
            && let Some(next) = next
        {
            tab.focused = next;
        }
        self.shutdown_agent_pane(pane, cx);
        self.panes.remove(&pane);
        if active {
            self.focus_active(window, cx);
        }
        self.announce_close(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Close tab `index`, asking first when any of its panes holds unsaved
    /// edits or an agent at work.
    pub(super) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.space().tabs.get(index) else {
            return;
        };
        let unsaved = self.unsaved_titles(tab.tree.leaves(), cx);
        let busy = self.busy_titles(tab.tree.leaves());
        if unsaved.is_empty() && busy.is_empty() {
            self.close_tab_now(index, window, cx);
        } else {
            let id = tab.id;
            self.confirm_close(unsaved, busy, window, cx, move |this, window, cx| {
                // The tab may have moved while the dialog was open.
                if let Some(index) = this.space().tabs.iter().position(|t| t.id == id) {
                    this.close_tab_now(index, window, cx);
                }
            });
        }
    }

    pub(super) fn close_tab_now(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.space().tabs.len() {
            return;
        }
        let leaves = self.space().tabs[index].tree.leaves();
        self.remember_close(index, &leaves, cx);
        let tab = self.space_mut().tabs.remove(index);
        for pane in tab.tree.leaves() {
            self.shutdown_agent_pane(pane, cx);
            self.panes.remove(&pane);
        }
        self.announce_close(window, cx);
        if self.space().tabs.is_empty() {
            // Machine navigation has an explicit + for new agents. Closing
            // the last agent must not silently spawn a replacement, local
            // or remote, which could surprise users after a disconnect.
            self.save_now(cx);
            cx.notify();
            return;
        }
        let active = self.space().active;
        let next = if active > index { active - 1 } else { active };
        self.activate(next, window, cx);
    }

    pub(super) fn swap_panes(&mut self, a: PaneId, b: PaneId, cx: &mut Context<Self>) {
        if a == b {
            return;
        }
        if let Some(tab) = self.tab_mut()
            && tab.tree.contains(&a)
            && tab.tree.contains(&b)
        {
            tab.tree.swap(&a, &b);
            self.schedule_save(cx);
            cx.notify();
        }
    }
}
