//! Screens drawn over the workspace: palette, settings, providers, sessions,
//! usage, memory, profiles, onboarding; and the surge. Floating web and
//! device cards live in `stages.rs`.

use super::*;

impl Workspace {
    // ── Screens ─────────────────────────────────────────────────────────────

    /// Open the switcher on `category`; the same shortcut again closes it, the
    /// other one changes category.
    pub(super) fn open_palette(
        &mut self,
        category: Category,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((Overlay::Palette(view), _)) = &self.overlay {
            if view.read(cx).category() == category {
                self.close_overlay(window, cx);
            } else {
                view.update(cx, |view, cx| view.set_category(category, window, cx));
            }
            return;
        }
        self.start_active_session(window, cx);
        let commands = self
            .active_session()
            .map(|t| t.read(cx).commands())
            .unwrap_or_default();
        let workspaces = self.workspace_rows();
        // An active remote agent is authoritative even if its bridge is
        // currently disconnected. Never index the Windows working tree as
        // though it were the remote agent's Files/Branches.
        let remote = if self.active_agent_is_remote(cx) {
            self.selected_remote_files(cx).map(|(session, machine, root)| {
                PaletteRemote {
                    machine,
                    root,
                    terminal: session.read(cx).herdr_terminal().unwrap_or_default(),
                    process: session.read(cx).remote_fs_process(),
                }
            }).or_else(|| Some(PaletteRemote {
                machine: String::new(),
                root: String::new(),
                terminal: String::new(),
                process: None,
            }))
        } else {
            None
        };
        let input = PaletteInput {
            commands,
            workspaces,
            root: self.space().root.clone(),
            remote,
            category,
        };
        let view = cx.new(|cx| PaletteView::new(input, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &PaletteEvent, window, cx| match event {
                PaletteEvent::Close => this.close_overlay(window, cx),
                PaletteEvent::Run(PaletteAction::RenameWorkspace { index, name }) => {
                    // Renaming happens in place; the switcher stays open.
                    this.rename_workspace(*index, name.clone(), cx);
                }
                PaletteEvent::Run(action) => {
                    this.close_overlay(window, cx);
                    this.run_palette_action(action.clone(), window, cx);
                }
            },
        );
        self.overlay = Some((Overlay::Palette(view), subscription));
        cx.notify();
    }

    /// Every workspace as the switcher lists it, with what its agents do.
    pub(super) fn workspace_rows(&self) -> Vec<WorkspaceRow> {
        self.spaces
            .iter()
            .enumerate()
            .map(|(index, space)| WorkspaceRow {
                index,
                name: space.name.clone(),
                tabs: space.tab_count(),
                current: index == self.current,
                activity: self.space_activity(space),
            })
            .collect()
    }

    pub(super) fn run_palette_action(
        &mut self,
        action: PaletteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = self.active_session();
        match action {
            PaletteAction::Providers => self.open_providers(window, cx),
            PaletteAction::Sessions => self.open_sessions(window, cx),
            PaletteAction::Machines => self.open_machines(window, cx),
            PaletteAction::Profiles => self.open_profiles(window, cx),
            PaletteAction::Usage => self.open_usage(window, cx),
            PaletteAction::Memory => self.open_memory(window, cx),
            PaletteAction::Docs => self.open_docs(window, cx),
            PaletteAction::History => {
                if self.active_agent_is_remote(cx) {
                    self.open_remote_history(window, cx);
                } else {
                    let cwd = self.active_cwd(cx);
                    self.open_history(cwd, window, cx);
                }
            }
            PaletteAction::Devices => self.devices.update(cx, |panel, cx| panel.show(cx)),
            PaletteAction::Settings => self.open_settings(window, cx),
            PaletteAction::Setup => self.open_onboarding(None, window, cx),
            PaletteAction::Welcome => self.open_welcome(None, window, cx),
            PaletteAction::NewTab => self.on_new_tab(&NewTab, window, cx),
            PaletteAction::NewTerminalTab => self.on_new_terminal_tab(&NewTerminalTab, window, cx),
            PaletteAction::ToggleAgentTerminal => self.toggle_agent_terminal(window, cx),
            PaletteAction::TilePane => self.tile(None, window, cx),
            PaletteAction::SplitRight => self.split(SplitAxis::Horizontal, None, window, cx),
            PaletteAction::SplitDown => self.split(SplitAxis::Vertical, None, window, cx),
            PaletteAction::NewAgentPane => self.split_with_kind(PaneKind::Agent, window, cx),
            PaletteAction::NewTerminalPane => self.split_with_kind(PaneKind::Terminal, window, cx),
            PaletteAction::ClosePane => self.on_close_pane(&ClosePane, window, cx),
            PaletteAction::ReopenClosed => self.reopen_closed(window, cx),
            PaletteAction::ToggleSidebar => self.toggle_sidebar(cx),
            PaletteAction::Agents => {
                if let Some(tab) = tab {
                    tab.update(cx, |s, cx| s.open_agents_panel(AgentsTab::Agents, cx));
                }
            }
            PaletteAction::NewSession => {
                if let Some(tab) = tab {
                    tab.update(cx, |s, cx| s.new_session(window, cx));
                }
            }
            PaletteAction::Command(name) => {
                if let Some(tab) = tab {
                    tab.update(cx, |s, cx| s.run_prompt(format!("/{name}"), cx));
                }
            }
            PaletteAction::Screen(screen) => self.open_screen(screen, tab, window, cx),
            PaletteAction::SwitchWorkspace(index) => self.switch_workspace(index, window, cx),
            PaletteAction::RenameWorkspace { index, name } => {
                self.rename_workspace(index, name, cx)
            }
            PaletteAction::DeleteWorkspace(index) => {
                self.confirm_delete_workspace(index, window, cx)
            }
            PaletteAction::CreateWorkspace {
                name,
                pick_folder: false,
            } => {
                let root = self.active_cwd(cx);
                self.create_workspace(Some(name), root, window, cx);
            }
            PaletteAction::CreateWorkspace {
                name,
                pick_folder: true,
            } => self.pick_workspace_folder(name, window, cx),
            PaletteAction::OpenFile(path) => {
                if self.active_agent_is_remote(cx) {
                    window.push_notification(Notification::info(
                        "Local file results cannot be opened for a remote agent. Reopen the palette.",
                    ), cx);
                } else {
                    self.open_file(path, window, cx);
                }
            }
            PaletteAction::OpenRemoteFile { machine, root, terminal, path } => {
                match self.selected_remote_files(cx) {
                    Some((session, current_machine, current_root))
                        if !terminal.is_empty() && machine == current_machine
                            && root == current_root
                            && session.read(cx).herdr_terminal().as_deref() == Some(terminal.as_str()) =>
                            self.open_remote_file(path, window, cx),
                    _ => window.push_notification(Notification::info(
                        "The active machine changed. Reopen the remote Files palette.",
                    ), cx),
                }
            }
            PaletteAction::SwitchBranch(branch) => {
                if self.active_agent_is_remote(cx) {
                    window.push_notification(Notification::info(
                        "Local Git cannot switch a remote branch. Reopen the remote palette.",
                    ), cx);
                } else {
                    self.confirm_switch_branch(branch, window, cx);
                }
            }
            PaletteAction::SwitchRemoteBranch { machine, root, terminal, branch } => {
                self.select_palette_remote_branch(&machine, &root, &terminal, branch, window, cx);
            }
        }
    }

    /// Play an effort level's surge over the window; another change
    /// restarts it at the new level.
    pub(super) fn play_surge(
        &mut self,
        level: crate::shared::surge::Level,
        cx: &mut Context<Self>,
    ) {
        self.surge_runs += 1;
        let run = self.surge_runs;
        self.surge = Some((run, std::time::Instant::now(), level));
        self._surge_end = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(level.duration()).await;
            let _ = this.update(cx, |this, cx| {
                if this.surge.is_some_and(|(playing, _, _)| playing == run) {
                    this.surge = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// The Agents window of `session` (else the active one), on `tab`.
    fn open_agents_window(
        &mut self,
        tab: AgentsTab,
        session: Option<Entity<SessionView>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = session.or_else(|| self.active_session()) {
            session.update(cx, |s, cx| s.open_agents_panel(tab, cx));
        }
    }

    /// The native screen of a terminal-only Gentle Shell command; `session`
    /// is the agent it concerns (the one that ran it, else the active one).
    pub(super) fn open_screen(
        &mut self,
        screen: Screen,
        session: Option<Entity<SessionView>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Slash commands belong to their originating Pi session. A screen
        // event arriving late from another tab must not open the Windows
        // or JaviPi resources belonging to the newly focused tab.
        if let Some(origin) = session.as_ref()
            && self.active_session().as_ref() != Some(origin)
        {
            window.push_notification(Notification::info(
                "Select the agent that requested this screen before opening it.",
            ), cx);
            return;
        }
        match screen {
            Screen::Usage => self.open_usage(window, cx),
            Screen::Stats => {
                self.open_usage(window, cx);
                if let Some((Overlay::Usage(view), _)) = &self.overlay {
                    view.update(cx, |view, cx| view.show_stats(cx));
                }
            }
            Screen::Agents => self.open_agents_window(AgentsTab::Agents, session, cx),
            // Background jobs are the Jobs tab of the Agents window.
            Screen::Jobs => self.open_agents_window(AgentsTab::Jobs, session, cx),
            Screen::Profiles => self.open_profiles(window, cx),
            Screen::Palette => {
                self.close_overlay(window, cx);
                self.open_palette(Category::Commands, window, cx);
            }
            Screen::Changes => {
                let target = session.or_else(|| self.active_session());
                // An event emitted by an inactive local *or* remote agent
                // must never redirect the active machine's Git dock.
                if target != self.active_session() {
                    window.push_notification(Notification::info(
                        "Select the originating agent before opening its Changes.",
                    ), cx);
                    return;
                }
                if target.as_ref().is_some_and(|agent| agent.read(cx).is_remote_herdr()) {
                    self.show_selected_remote_changes(window, cx);
                } else {
                    let Some(tree) = self
                        .sidebar
                        .clone()
                        .and_then(|view| view.downcast::<FileTreeView>().ok())
                    else {
                        return;
                    };
                    if !self.files_sidebar_open {
                        self.toggle_files_panel(cx);
                    }
                    tree.update(cx, |tree, cx| tree.show_changes(window, cx));
                }
            }
        }
    }

    /// Pick a Linux working directory before creating a remote agent. No
    /// remote terminal is started while the picker is open or cancelled.
    pub(super) fn open_remote_directory_picker(
        &mut self,
        machine: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !crate::features::terminal::herdr_enabled() {
            return;
        }
        if self.machine_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.machines.iter().any(|item| {
                item.name == machine && !item.connected()
            })
        }) {
            window.push_notification(Notification::info(
                format!("{machine} is disconnected. No agent was started."),
            ), cx);
            return;
        }
        // Use Herdr's known terminal directories only as shortcuts: they
        // are not a complete remote filesystem listing.
        let snapshot = self.machine_snapshot.clone();
        let view = cx.new(|cx| RemoteDirectoryPicker::new(
            machine, snapshot, window, cx,
        ));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &RemoteDirectoryEvent, window, cx| match event {
                RemoteDirectoryEvent::Close => this.close_overlay(window, cx),
                RemoteDirectoryEvent::Launch { machine, cwd } => {
                    // Defense in depth: the picker already validated the
                    // path, but the workspace is the final launch boundary.
                    let target = pi_rpc::AgentTarget::remote_with_cwd(
                        machine.clone(), cwd.clone(),
                    );
                    this.close_overlay(window, cx);
                    match target {
                        Ok(target) => this.open_agent_on_machine(target, window, cx),
                        Err(error) => window.push_notification(
                            Notification::info(format!("Invalid remote directory: {error}")),
                            cx,
                        ),
                    }
                }
            },
        );
        self.overlay = Some((Overlay::RemoteDirectory(view), subscription));
        cx.notify();
    }

    /// Herdr machine and terminal browser (all discovery/actions off the UI thread).
    pub(super) fn open_machines(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_machines_impl(false, window, cx);
    }

    /// The + in the Machines navigator registers an SSH machine in Herdr.
    /// Creating an agent remains an explicit per-machine action.
    pub(super) fn open_machine_registration(
        &mut self, window: &mut Window, cx: &mut Context<Self>,
    ) {
        self.open_machines_impl(true, window, cx);
    }

    fn open_machines_impl(
        &mut self, register: bool, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if !crate::features::terminal::herdr_enabled() {
            return;
        }
        if matches!(&self.overlay, Some((Overlay::Machines(_), _))) {
            if register {
                if let Some((Overlay::Machines(view), _)) = &self.overlay {
                    view.update(cx, |view, cx| view.begin_add_machine(window, cx));
                }
            } else {
                self.close_overlay(window, cx);
            }
            return;
        }
        let view = cx.new(|cx| {
            if register {
                MachinesView::new_for_registration(window, cx)
            } else {
                MachinesView::new(window, cx)
            }
        });
        view.update(cx, |view, cx| {
            view.set_known_agent_addresses(self.known_herdr_agent_addresses(cx), cx)
        });
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &MachinesEvent, window, cx| match event {
                MachinesEvent::Close => this.close_overlay(window, cx),
                MachinesEvent::OpenTerminal(address) => {
                    let address = address.clone();
                    this.close_overlay(window, cx);
                    this.show_or_attach_herdr_terminal(address, window, cx);
                }
                MachinesEvent::NewAgent(machine) => {
                    let machine = machine.clone();
                    this.close_overlay(window, cx);
                    match machine {
                        Some(machine) => {
                            this.open_remote_directory_picker(machine, window, cx)
                        }
                        None => this.pick_local_agent_folder(window, cx),
                    }
                }
            },
        );
        self.overlay = Some((Overlay::Machines(view), subscription));
        cx.notify();
    }

    pub(super) fn open_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_agent_is_remote(cx) {
            window.push_notification(Notification::info(
                "Saved Sessions browsing is local to this computer. Showing live Herdr machines and terminals; no Windows session will be opened on JaviPi.",
            ), cx);
            self.open_machines(window, cx);
            return;
        }
        self.start_active_session(window, cx);
        let cwd = self.active_cwd(cx);
        let current = self
            .active_session()
            .and_then(|t| t.read(cx).session_file());
        let view = cx.new(|cx| SessionsView::new(cwd, current, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SessionsEvent, window, cx| match event {
                SessionsEvent::Close => this.close_overlay(window, cx),
                SessionsEvent::NewSession => {
                    this.close_overlay(window, cx);
                    if let Some(tab) = this.active_session() {
                        tab.update(cx, |s, cx| s.new_session(window, cx));
                    }
                }
                SessionsEvent::Open { path, cwd } => {
                    this.close_overlay(window, cx);
                    let path = path.clone();
                    if *cwd == this.active_cwd(cx) {
                        if let Some(tab) = this.active_session() {
                            tab.update(cx, |s, cx| s.open_session(path, cx));
                        }
                    } else {
                        this.open_tab(PaneKind::Agent, cwd.clone(), None, Some(path), window, cx);
                    }
                }
            },
        );
        self.overlay = Some((Overlay::Sessions(view), subscription));
        cx.notify();
    }

    pub(super) fn open_profiles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_active_session(window, cx);
        let cwd = self.active_cwd(cx);
        let models = self
            .active_session()
            .map(|t| t.read(cx).model_choices())
            .unwrap_or_default()
            .into_iter()
            .map(|(provider, id, name, reasoning)| ModelChoice {
                provider,
                id,
                name,
                reasoning,
            })
            .collect();
        let session = self.active_session();
        let owner = session.clone();
        let view = cx.new(|cx| ProfilesView::new(cwd, models, session, window, cx));
        let subscription = cx.subscribe_in(&view, window, move |this, _, event: &ProfilesEvent, window, cx| {
            // Profile actions must reach the session that opened the screen,
            // never whichever machine happened to gain focus since then.
            let tab = owner.clone();
            match event {
                ProfilesEvent::Close => this.close_overlay(window, cx),
                ProfilesEvent::Changed => {}
                ProfilesEvent::RunCommand(text) => {
                    if let Some(tab) = tab {
                        let text = text.clone();
                        tab.update(cx, |s, cx| s.run_prompt(text, cx));
                    }
                }
                ProfilesEvent::SwitchOrchestrator { model, thinking } => {
                    if let Some(tab) = tab {
                        let ((provider, id), thinking) = (model.clone(), thinking.clone());
                        tab.update(cx, |s, cx| {
                            s.send_command(serde_json::json!({"type": "set_model", "provider": provider, "modelId": id}), cx);
                            if let Some(level) = thinking {
                                s.send_command(serde_json::json!({"type": "set_thinking_level", "level": level}), cx);
                            }
                        });
                    }
                }
                ProfilesEvent::UseInSession { profile, model, thinking } => {
                    if let Some(tab) = tab {
                        let (model, thinking) = (model.clone(), thinking.clone());
                        let note = format!("Profile \u{201c}{profile}\u{201d} applied to this session's orchestrator; helpers keep the routing this session started with.");
                        tab.update(cx, |s, cx| {
                            s.note(note, cx);
                            if let Some((provider, id)) = model {
                                s.send_command(serde_json::json!({"type": "set_model", "provider": provider, "modelId": id}), cx);
                            }
                            if let Some(level) = thinking {
                                s.send_command(serde_json::json!({"type": "set_thinking_level", "level": level}), cx);
                            }
                        });
                    }
                }
            }
        });
        self.overlay = Some((Overlay::Profiles(view), subscription));
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_agent_is_remote(cx) {
            window.push_notification(Notification::info(
                "Settings manages this Windows desktop and local Gentle Shell installation. Remote agent settings are not modified by this screen.",
            ), cx);
        }
        self.start_active_session(window, cx);
        let cwd = self.active_cwd(cx);
        let view = cx.new(|cx| SettingsView::new(cwd, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Close => this.close_overlay(window, cx),
                SettingsEvent::Changed => {
                    for tab in this.sessions() {
                        if !tab.read(cx).is_remote_herdr() {
                            tab.update(cx, |session, cx| session.reload_models(window, cx));
                        }
                    }
                }
                SettingsEvent::DevicesChanged(prefs) => {
                    let prefs = *prefs;
                    crate::features::devices::hub(cx)
                        .update(cx, |hub, cx| hub.set_prefs(prefs, cx));
                    pi_rpc::set_system_prompt(crate::features::devices::agent_instructions(prefs));
                    if !prefs.enabled {
                        this.devices.update(cx, |panel, cx| panel.hide(cx));
                    }
                    cx.notify();
                }
                SettingsEvent::MusicChanged => {
                    this.music.update(cx, |music, cx| music.refresh(cx));
                }
                SettingsEvent::BackendUpdated => {
                    for tab in this.sessions() {
                        if !tab.read(cx).is_remote_herdr() {
                            tab.update(cx, |session, cx| session.restart_for_update(window, cx));
                        }
                    }
                }
                SettingsEvent::Setup(event) => {
                    let usable = view.read(cx).setup_usable(cx);
                    this.on_setup_event(event, usable, window, cx);
                }
                SettingsEvent::ShellCommand(text) => {
                    let view = view.downgrade();
                    let report = move |result: Result<(), String>, cx: &mut App| {
                        let _ = view.update(cx, |view, cx| view.shell_done(result, cx));
                    };
                    match this.active_session() {
                        Some(session) if !session.read(cx).is_remote_herdr() =>
                            session.update(cx, |session, cx| {
                                session.run_extension_command(text.clone(), cx, report)
                            }),
                        _ => report(
                            Err(
                                "Open an agent tab first: Gentle Shell applies this setting."
                                    .into(),
                            ),
                            cx,
                        ),
                    }
                }
            },
        );
        self.overlay = Some((Overlay::Settings(view), subscription));
        cx.notify();
    }

    /// Where an open agent tab found the docs skill, if any did.
    pub(super) fn docs_skill_dir(&self, cx: &App) -> Option<PathBuf> {
        self.active_session()
            .into_iter()
            .chain(self.sessions())
            .find_map(|s| s.read(cx).skill_dir(crate::features::docs::SKILL))
    }

    pub(super) fn open_docs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_agent_is_remote(cx) {
            self.show_selected_remote_docs(window, cx);
            window.push_notification(Notification::info(
                "Showing Linux docs in Remote Files. Architecture Docs installation and checks run only in the local workspace; use the remote agent to generate or update docs.",
            ), cx);
            return;
        }
        let root = self.space().root.clone();
        let skill = self.docs_skill_dir(cx);
        let view = cx.new(|cx| DocsView::new(root.clone(), skill, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            move |this, _, event: &DocsEvent, window, cx| match event {
                DocsEvent::Close => this.close_overlay(window, cx),
                DocsEvent::Run(prompt) => {
                    this.close_overlay(window, cx);
                    this.open_tab(
                        PaneKind::Agent,
                        root.clone(),
                        Some(prompt.clone()),
                        None,
                        window,
                        cx,
                    );
                }
                DocsEvent::Open(path) => {
                    this.close_overlay(window, cx);
                    this.open_file(path.clone(), window, cx);
                }
            },
        );
        self.overlay = Some((Overlay::Docs(view), subscription));
        cx.notify();
    }

    pub(super) fn open_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_agent_is_remote(cx) {
            window.push_notification(Notification::info(
                "The Memory browser connects to this computer's Engram, not JaviPi. Query the remote agent for its own memories.",
            ), cx);
            return;
        }
        let cwd = self.active_cwd(cx);
        let view = cx.new(|cx| MemoryView::new(cwd, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &MemoryEvent, window, cx| match event {
                MemoryEvent::Close => this.close_overlay(window, cx),
            },
        );
        self.overlay = Some((Overlay::Memory(view), subscription));
        cx.notify();
    }

    pub(super) fn open_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The Subscription tab reads through the active agent's RPC slice, so
        // its backend must be started (probe included) before it is opened.
        self.start_active_session(window, cx);
        let session = self.active_session();
        let view = cx.new(|cx| UsageView::new(window, cx, session));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &UsageEvent, window, cx| match event {
                UsageEvent::Close => this.close_overlay(window, cx),
            },
        );
        self.overlay = Some((Overlay::Usage(view), subscription));
        cx.notify();
    }

    pub(super) fn open_providers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_agent_is_remote(cx) {
            window.push_notification(Notification::info(
                "Provider accounts here belong to the Windows installation. JaviPi uses its own credentials and provider configuration.",
            ), cx);
        }
        let cwd = self.active_cwd(cx);
        let view = cx.new(|cx| ProvidersView::new(cwd, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &ProvidersEvent, window, cx| match event {
                ProvidersEvent::Close => {
                    this.close_overlay(window, cx);
                    // Signed in from the welcome screen: back to its sequence.
                    if let Some(view) = this.resume_welcome.take() {
                        this.attach_welcome(view, window, cx);
                    }
                }
                ProvidersEvent::Changed => {
                    for tab in this.sessions() {
                        if !tab.read(cx).is_remote_herdr() {
                            tab.update(cx, |session, cx| session.reload_models(window, cx));
                        }
                    }
                }
            },
        );
        self.overlay = Some((Overlay::Providers(view), subscription));
        cx.notify();
    }

    /// The onboarding screen; `probe` is a look at the machine already taken.
    pub(super) fn open_onboarding(
        &mut self,
        probe: Option<Probe>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| OnboardingView::new(probe, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &OnboardingEvent, window, cx| {
                let usable = view.read(cx).usable();
                this.on_setup_event(event, usable, window, cx);
            },
        );
        self.overlay = Some((Overlay::Onboarding(view), subscription));
        cx.notify();
    }

    /// The first-run welcome screen; `probe` is a look at the machine already
    /// taken. ⌘K → "Welcome screen" shows it again.
    pub(super) fn open_welcome(
        &mut self,
        probe: Option<Probe>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resume_welcome = None;
        let view = cx.new(|cx| WelcomeView::new(probe, cx));
        self.attach_welcome(view, window, cx);
    }

    /// Show `view` (new, or back from Providers) and route its events.
    pub(super) fn attach_welcome(
        &mut self,
        view: Entity<WelcomeView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &WelcomeEvent, window, cx| match event {
                WelcomeEvent::Close => this.close_overlay(window, cx),
                // Providers takes the window; closing it returns to the welcome.
                WelcomeEvent::Setup(OnboardingEvent::SignIn) => {
                    this.resume_welcome = Some(view.clone());
                    this.close_overlay(window, cx);
                    this.open_providers(window, cx);
                }
                WelcomeEvent::Setup(event) => {
                    let usable = view.read(cx).usable(cx);
                    this.on_setup_event(event, usable, window, cx);
                }
            },
        );
        view.read(cx).focus_handle().clone().focus(window, cx);
        self.overlay = Some((Overlay::Welcome(view), subscription));
        cx.notify();
    }

    /// An action from the setup checklist, on the onboarding screen or in
    /// Configuration (which is closed first, like the screen, when the action
    /// moves elsewhere). `usable`: gentle-shell can start as of its last look.
    pub(super) fn on_setup_event(
        &mut self,
        event: &OnboardingEvent,
        usable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            OnboardingEvent::Close => self.close_overlay(window, cx),
            OnboardingEvent::Installed => {
                if usable {
                    self.retry_failed_sessions(window, cx);
                }
            }
            OnboardingEvent::SignIn => {
                self.close_overlay(window, cx);
                self.open_providers(window, cx);
            }
            OnboardingEvent::Terminal(command) => {
                self.close_overlay(window, cx);
                self.open_setup_terminal(command.clone(), window, cx);
            }
        }
    }

    /// A terminal tab with `command` typed at the prompt for the user to run
    /// (it may ask for a password); the clipboard holds it when the terminal
    /// can't take it.
    pub(super) fn open_setup_terminal(
        &mut self,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let home = dirs::home_dir().unwrap_or_else(|| self.active_cwd(cx));
        let Some(pane) = self.new_pane(PaneKind::Terminal, home, None, None, window, cx) else {
            cx.write_to_clipboard(ClipboardItem::new_string(command));
            window.push_notification(
                Notification::info("The setup command is on your clipboard: run it in a terminal."),
                cx,
            );
            return;
        };
        self.push_pane_tab(pane, window, cx);
        let terminal = self.panes.get(&pane).and_then(|p| {
            p.item
                .view
                .clone()
                .downcast::<crate::features::terminal::TerminalView>()
                .ok()
        });
        let note = match terminal {
            Some(terminal) => {
                terminal.update(cx, |terminal, cx| terminal.type_when_ready(command, cx));
                "The command is typed in this terminal: press Enter to run it. Reopen Setup & requirements from the command palette when it's done."
            }
            None => {
                cx.write_to_clipboard(ClipboardItem::new_string(command));
                "The setup command is on your clipboard: paste it here and press Enter. Reopen Setup & requirements from the command palette when it's done."
            }
        };
        window.push_notification(Notification::info(note), cx);
    }

    /// Start again every agent tab whose backend could not start (onboarding
    /// just made gentle-shell available).
    pub(super) fn retry_failed_sessions(&self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in self.sessions() {
            tab.update(cx, |session, cx| session.retry_start(window, cx));
        }
    }
}
