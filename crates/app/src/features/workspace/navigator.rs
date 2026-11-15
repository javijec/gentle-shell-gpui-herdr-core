//! Machine-first navigation. Tabs remain as internal session containers for
//! existing persistence, split panes and Herdr remote agent ownership, but
//! there is deliberately no visual tab strip.

use super::*;
use gpui_kit::component::Disableable as _;

#[derive(Clone)]
enum NavigatorSessionSource {
    /// One visible pane, which may be an agent or a terminal in a split.
    Open { space: usize, tab: usize, pane: PaneId },
    /// A session saved in a workspace that has not been restored yet.
    Saved { space: usize, tab: usize, pane_index: usize },
    /// A Herdr terminal not yet represented by any GPUI pane.
    Herdr { address: String, live: bool, agent: bool },
}

struct NavigatorSession {
    source: NavigatorSessionSource,
    row_id: String,
    title: String,
    address: Option<String>,
    /// Real machine working directory; never substitute a Windows cwd for a Pi.
    directory: Option<String>,
    kind: &'static str,
    icon: IconName,
    running: bool,
    pane_count: usize,
}

struct NavigatorMachine {
    key: String,
    name: String,
    connected: bool,
    /// Reported by the owning Herdr Core; saved-only machines are "unknown".
    state: String,
    local: bool,
    sessions: Vec<NavigatorSession>,
}

/// Display only states reported by Herdr. A saved session is not proof that
/// its machine is currently online.
fn navigator_machine_status(state: &str) -> &'static str {
    match state {
        "connected" => "Connected",
        "connecting" => "Connecting",
        "needs_login" => "Needs login",
        "needs_update" => "Needs update",
        "unreachable" => "Unreachable",
        _ => "Status unknown",
    }
}

/// An explicit 4C target is authoritative. An older 4B persisted address
/// provides a machine only if the target wasn't recorded in that workspace.
fn machine_key(
    target: Option<&pi_rpc::AgentTarget>,
    address: Option<&str>,
) -> Option<String> {
    match target {
        Some(pi_rpc::AgentTarget::Local) => None,
        Some(pi_rpc::AgentTarget::Remote { machine, .. }) => Some(machine.clone()),
        None => address.and_then(|value| value.split_once('/'))
            .map(|(machine, _)| machine.to_string()),
    }
}

/// Resolve the runtime directory independently of the Windows workspace
/// folder. Herdr terminal metadata is preferred once available; for new
/// remote agents, the stored per-agent target is the launch directory.
/// An old remote address without metadata yields None, never a fake
/// Windows path.
fn session_directory(
    target: Option<&pi_rpc::AgentTarget>,
    address: Option<&str>,
    local_cwd: &Path,
    snapshot: Option<&machine_model::Snapshot>,
) -> Option<String> {
    let reported = snapshot.and_then(|snapshot| {
        snapshot
            .terminals
            .iter()
            .find(|terminal| Some(terminal.address.as_str()) == address)
            .map(|terminal| terminal.cwd.trim())
            .filter(|cwd| !cwd.is_empty())
            .map(str::to_owned)
    });
    match target {
        Some(pi_rpc::AgentTarget::Local) => Some(local_cwd.display().to_string()),
        Some(pi_rpc::AgentTarget::Remote { cwd, .. }) => {
            reported.or_else(|| Some(cwd.clone()))
        }
        None if address.is_some_and(|address| address.contains('/')) => reported,
        None => reported.or_else(|| Some(local_cwd.display().to_string())),
    }
}

impl Workspace {
    /// The focused agent or terminal owns the active machine. A focused
    /// auxiliary File or History pane does not pretend to be a local session:
    /// use the first agent or terminal from the same tab instead.
    fn selected_runtime_pane(&self) -> Option<&super::Pane> {
        let tab = self.tab()?;
        let is_runtime = |pane: &&super::Pane| {
            pane.item.kind == PaneKind::Agent || pane.item.kind == PaneKind::Terminal
        };
        self.panes.get(&tab.focused).filter(is_runtime)
            .or_else(|| tab.tree.leaves().into_iter()
                .find_map(|id| self.panes.get(&id).filter(is_runtime)))
    }

    /// Show the same owner as the active session title label.
    fn active_navigator_machine_key(&self) -> String {
        if !crate::features::terminal::herdr_enabled() {
            return "local".into();
        }
        self.selected_runtime_pane().and_then(|pane| {
            machine_key(pane.agent_target.as_ref(), pane.herdr_address.as_deref())
        }).unwrap_or_else(|| "local".into())
    }

    /// Selecting a computer in the compact rail expands *that* group,
    /// without switching sessions, spawning agents or losing the saved width.
    fn expand_machine_from_rail(&mut self, key: String, cx: &mut Context<Self>) {
        self.collapsed_machines.remove(&key);
        if !self.sidebar_visible {
            self.sidebar_visible = true;
            self.schedule_save(cx);
        }
        cx.notify();
    }

    /// Machine and runtime path for the active agent or terminal, independent
    /// from the Windows workspace name in the title bar.
    pub(super) fn active_runtime_label(&self) -> String {
        let Some(pane) = self.selected_runtime_pane() else {
            return "No agent or terminal selected".into();
        };
        let herdr = crate::features::terminal::herdr_enabled();
        let machine = if herdr {
            machine_key(pane.agent_target.as_ref(), pane.herdr_address.as_deref())
                .unwrap_or_else(|| "This computer".into())
        } else {
            "This computer".into()
        };
        let directory = if herdr {
            session_directory(
                pane.agent_target.as_ref(),
                pane.herdr_address.as_deref(),
                &pane.cwd,
                self.machine_snapshot.as_ref(),
            )
        } else {
            Some(pane.cwd.display().to_string())
        };
        format!("{machine} · {}", directory.unwrap_or_else(|| "remote cwd unavailable".into()))
    }

    /// Build the machine tree from both sources of truth: GPUI panes (agents
    /// and terminals, including splits and other workspaces) and Herdr Core's
    /// terminal.list. Every Herdr address appears only once.
    fn navigator_groups(&self) -> Vec<NavigatorMachine> {
        let mut groups = vec![NavigatorMachine {
            key: "local".into(),
            name: "This computer".into(),
            connected: true,
            state: "connected".into(),
            local: true,
            sessions: Vec::new(),
        }];
        if let Some(snapshot) = &self.machine_snapshot {
            for machine in &snapshot.machines {
                if !groups.iter().any(|group| group.key == machine.name) {
                    groups.push(NavigatorMachine {
                        key: machine.name.clone(),
                        name: machine.name.clone(),
                        connected: machine.connected(),
                        state: machine.state.clone(),
                        local: false,
                        sessions: Vec::new(),
                    });
                }
            }
        }

        let mut shown = std::collections::HashSet::new();
        let mut add = |key: String, entry: NavigatorSession| {
            if let Some(address) = &entry.address
                && (self.terminated_herdr_sessions.contains(address)
                    || !shown.insert(address.clone()))
            {
                return;
            }
            let index = match groups.iter().position(|group| group.key == key) {
                Some(index) => index,
                None => {
                    // A recovered/saved session can exist while discovery is
                    // offline. Keep it visible without claiming connectivity.
                    let local = key == "local";
                    groups.push(NavigatorMachine {
                        name: if local { "This computer".into() } else { key.clone() },
                        key,
                        connected: local,
                        state: if local { "connected" } else { "unknown" }.into(),
                        local,
                        sessions: Vec::new(),
                    });
                    groups.len() - 1
                }
            };
            groups[index].sessions.push(entry);
        };

        for (space_index, space) in self.spaces.iter().enumerate() {
            for (tab_index, tab) in space.tabs.iter().enumerate() {
                // Render EACH agent and terminal pane, not only the first
                // agent of a multi-pane tab.
                for (pane_index, pane_id) in tab.tree.leaves().into_iter().enumerate() {
                    let Some(pane) = self.panes.get(&pane_id) else { continue };
                    let kind = match pane.item.kind {
                        PaneKind::Agent => "Gentle Shell",
                        PaneKind::Terminal => "Terminal",
                        _ => continue,
                    };
                    // The title poll keeps this identity current. Do not
                    // read a session entity during the GPUI render pass.
                    let address = pane.herdr_address.clone();
                    let key = if crate::features::terminal::herdr_enabled() {
                        machine_key(pane.agent_target.as_ref(), address.as_deref())
                            .unwrap_or_else(|| "local".into())
                    } else {
                        "local".into()
                    };
                    let title = pane.title.to_string();
                    let directory = session_directory(
                        pane.agent_target.as_ref(), address.as_deref(),
                        &pane.cwd, self.machine_snapshot.as_ref(),
                    );
                    add(key, NavigatorSession {
                        source: NavigatorSessionSource::Open {
                            space: space_index, tab: tab_index,
                            pane: pane_id,
                        },
                        row_id: format!("open-{space_index}-{}-{pane_index}", tab.id),
                        title: if title.is_empty() { kind.into() } else { title },
                        address,
                        directory,
                        kind,
                        icon: pane.item.kind.icon(),
                        running: self.tab_running(tab),
                        pane_count: tab.tree.len(),
                    });
                }
            }
        }

        // Workspaces are lazy. Include their saved agent/terminal panes even
        // when they have not yet been instantiated in GPUI.
        for (space_index, space) in self.spaces.iter().enumerate() {
            let Some((tabs, _)) = &space.saved else { continue };
            for (tab_index, tab) in tabs.iter().enumerate() {
                for (pane_index, pane) in tab.layout.leaves().iter().enumerate() {
                    let kind = match pane.kind {
                        PaneKind::Agent => "Gentle Shell",
                        PaneKind::Terminal => "Terminal",
                        _ => continue,
                    };
                    let address = pane.herdr_terminal.clone();
                    let key = if crate::features::terminal::herdr_enabled() {
                        machine_key(pane.agent_target.as_ref(), address.as_deref())
                            .unwrap_or_else(|| "local".into())
                    } else {
                        "local".into()
                    };
                    add(key, NavigatorSession {
                        source: NavigatorSessionSource::Saved {
                            space: space_index, tab: tab_index, pane_index,
                        },
                        row_id: format!("saved-{space_index}-{tab_index}-{pane_index}"),
                        title: format!("{kind} (saved)"),
                        directory: session_directory(
                            pane.agent_target.as_ref(), address.as_deref(),
                            &pane.cwd, self.machine_snapshot.as_ref(),
                        ),
                        address,
                        kind,
                        icon: pane.kind.icon(),
                        running: false,
                        pane_count: tab.layout.leaves().len(),
                    });
                }
            }
        }

        if crate::features::terminal::herdr_enabled()
            && let Some(snapshot) = &self.machine_snapshot
        {
            for terminal in &snapshot.terminals {
                let key = terminal.address.split_once('/')
                    .map_or_else(|| "local".to_owned(), |(machine, _)| machine.to_owned());
                let live = terminal.live();
                add(key, NavigatorSession {
                    source: NavigatorSessionSource::Herdr {
                        address: terminal.address.clone(), live,
                        agent: terminal.gentle_shell,
                    },
                    row_id: format!("herdr-{}", terminal.address),
                    // The terminal's Herdr labels distinguish bridge agents
                    // from shells, including agents not yet opened in GPUI.
                    title: terminal.display_title().to_owned(),
                    address: Some(terminal.address.clone()),
                    directory: (!terminal.cwd.is_empty())
                        .then(|| terminal.cwd.clone()),
                    kind: if terminal.gentle_shell { "Gentle Shell" } else { "Terminal" },
                    icon: if terminal.gentle_shell {
                        IconName::Bot
                    } else {
                        IconName::SquareTerminal
                    },
                    running: live,
                    pane_count: 1,
                });
            }
        }
        groups
    }

    /// Close from Machines means permanent termination, not merely detaching
    /// the GPUI viewer or saving an Undo snapshot. Always ask first.
    fn confirm_permanent_session_close(
        &mut self, source: NavigatorSessionSource,
        address: Option<String>, agent: bool,
        window: &mut Window, cx: &mut Context<Self>,
    ) {
        if address.as_ref().is_some_and(|address| {
            self.terminating_herdr_sessions.contains(address)
                || self.terminated_herdr_sessions.contains(address)
        }) {
            return;
        }
        if address.is_some() && !crate::features::terminal::herdr_enabled() {
            window.push_notification(Notification::info(
                "Switch to the Herdr backend to permanently terminate an existing Herdr session.",
            ), cx);
            return;
        }
        let kind = if agent { "Gentle Shell agent" } else { "terminal" };
        let title = format!("Terminate {kind} permanently?");
        let description = format!(
            "This stops the {kind} on its computer and removes it from Machines. \
             It will not be restored on the next launch. This action cannot be undone. \
             Saved conversation files are not deleted. Unsaved edits in attached \
             file panes may be lost."
        );
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            let source = source.clone();
            let address = address.clone();
            alert
                .title(title.clone())
                .description(description.clone())
                .ok_text("Terminate permanently")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.permanently_close_machine_session(
                            source.clone(), address.clone(), agent, window, cx,
                        );
                    });
                    true
                })
        });
    }

    fn permanently_close_machine_session(
        &mut self, source: NavigatorSessionSource,
        address: Option<String>, agent: bool,
        window: &mut Window, cx: &mut Context<Self>,
    ) {
        if let Some(address) = address {
            if self.terminated_herdr_sessions.contains(&address)
                || !self.terminating_herdr_sessions.insert(address.clone())
            {
                return;
            }
            cx.notify();
            let task = cx.background_executor().spawn({
                let address = address.clone();
                async move { machine_model::terminate_session(&address, agent) }
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.terminating_herdr_sessions.remove(&address);
                    match result {
                        Ok(()) => {
                            this.terminated_herdr_sessions.insert(address.clone());
                            this.remove_machine_session_from_desktop(
                                &source, Some(&address), window, cx,
                            );
                            this.save_now(cx);
                            window.push_notification(Notification::info(
                                "Session terminated in Herdr and removed from Machines.",
                            ), cx);
                        }
                        Err(error) => {
                            window.push_notification(Notification::error(format!(
                                "Could not terminate Herdr session {address}: {error}"
                            )), cx);
                        }
                    }
                    cx.notify();
                });
            }).detach();
        } else {
            // A local agent may be pending launch and have no address yet.
            // There is no Herdr terminal to kill; disposing the pane stops
            // its owned Pi backend and prevents subsequent restore.
            self.remove_machine_session_from_desktop(&source, None, window, cx);
            self.save_now(cx);
        }
    }

    /// Remove all desktop references to a terminated session. This also
    /// clears its linked remote file/history panes and stale saved layouts.
    /// Unlike close_tab(), nothing enters the Undo/reopen queue.
    fn remove_machine_session_from_desktop(
        &mut self,
        source: &NavigatorSessionSource,
        address: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_pane = match source {
            NavigatorSessionSource::Open { pane, .. } => Some(*pane),
            _ => None,
        };
        let selected_saved = match source {
            NavigatorSessionSource::Saved { space, tab, pane_index } =>
                Some((*space, *tab, *pane_index)),
            _ => None,
        };
        let matching: std::collections::HashSet<PaneId> = self.panes.iter()
            .filter_map(|(id, pane)| {
                let same_address = address.is_some_and(|address| {
                    pane.herdr_address.as_deref() == Some(address)
                        || pane.item.terminal_address.as_ref()
                            .and_then(|get| get(cx)).as_deref() == Some(address)
                        || (matches!(pane.item.kind, PaneKind::File | PaneKind::History)
                            && pane.linked_session.as_ref()
                                .and_then(|session| session.read(cx).herdr_terminal())
                                .as_deref() == Some(address))
                });
                (same_address || selected_pane == Some(*id)).then_some(*id)
            })
            .collect();

        for id in &matching {
            self.shutdown_agent_pane(*id, cx);
        }
        for (space_index, space) in self.spaces.iter_mut().enumerate() {
            space.tabs.retain_mut(|tab| {
                let Some(tree) = tab.tree.filter_map(&mut |id| {
                    (!matching.contains(id)).then_some(*id)
                }) else {
                    return false;
                };
                if !tree.contains(&tab.focused) {
                    tab.focused = tree.leaves()[0];
                }
                tab.tree = tree;
                true
            });
            space.active = space.active.min(space.tabs.len().saturating_sub(1));
            if let Some((tabs, active)) = &mut space.saved {
                let mut tab_index = 0;
                tabs.retain_mut(|tab| {
                    let original_tab = tab_index;
                    tab_index += 1;
                    let mut pane_index = 0;
                    let Some(tree) = tab.layout.filter_map(&mut |pane: &SavedPane| {
                        let original_pane = pane_index;
                        pane_index += 1;
                        let same_address = address.is_some_and(|addr| {
                            pane.herdr_terminal.as_deref() == Some(addr)
                                || (matches!(pane.kind, PaneKind::File | PaneKind::History)
                                    && pane.remote_owner.as_deref() == Some(addr))
                        });
                        let selected = selected_saved == Some((
                            space_index, original_tab, original_pane,
                        ));
                        (!same_address && !selected).then(|| {
                            let mut kept = pane.clone();
                            if address.is_some_and(|addr| {
                                kept.remote_owner.as_deref() == Some(addr)
                            }) {
                                kept.remote_owner = None;
                            }
                            kept
                        })
                    }) else {
                        return false;
                    };
                    tab.focused = tab.focused.min(tree.leaves().len().saturating_sub(1));
                    tab.layout = tree;
                    true
                });
                *active = (*active).min(tabs.len().saturating_sub(1));
            }
        }
        for id in matching {
            self.panes.remove(&id);
        }
        // Independent terminals may have been opened beside the agent.
        // Keep their own Herdr sessions alive and drop only the dead parent link.
        for pane in self.panes.values_mut() {
            if address.is_some_and(|addr| {
                pane.linked_session.as_ref()
                    .and_then(|session| session.read(cx).herdr_terminal())
                    .as_deref() == Some(addr)
            }) {
                pane.linked_session = None;
            }
        }
        // Undo records intentionally omit dead Herdr addresses, so there is
        // no reliable way to pick only this one's old close snapshots.
        // Clearing them ensures Undo cannot resurrect a terminated agent.
        self.closed.clear();
        window.remove_notification::<closed::Closed>(cx);
        self.focus_active(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn toggle_machine_group(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.collapsed_machines.insert(key.clone()) {
            self.collapsed_machines.remove(&key);
        }
        cx.notify();
    }

    fn new_agent_in_group(
        &mut self,
        machine: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match machine {
            Some(machine) if crate::features::terminal::herdr_enabled() => {
                self.open_remote_directory_picker(machine, window, cx);
            }
            None => self.pick_local_agent_folder(window, cx),
            Some(_) => {}
        }
    }

    /// Start a Herdr terminal on the requested machine, independent of the
    /// currently selected agent. Never spawn Pi when Terminal is selected.
    fn new_terminal_in_group(
        &mut self,
        machine: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if crate::features::terminal::herdr_enabled() {
            if let Some(name) = &machine
                && !self.machine_snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.machines.iter().any(|item| {
                        item.name == *name && item.connected()
                    })
                })
            {
                window.push_notification(Notification::info(
                    format!("{name} is disconnected. No terminal was created."),
                ), cx);
                return;
            }
            let task = cx.background_executor().spawn(async move {
                machine_model::create_shell(machine.as_deref())
            });
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(address) => this.open_herdr_terminal(address, window, cx),
                    Err(error) => window.push_notification(
                        Notification::error(error), cx,
                    ),
                });
            }).detach();
        } else if machine.is_none() {
            let cwd = self.active_cwd(cx);
            self.open_tab(PaneKind::Terminal, cwd, None, None, window, cx);
        }
    }

    /// Request an immediate reconnect for a registered machine without
    /// creating a terminal or altering any agent viewer. Herdr Core owns the
    /// transport, retry policy and authentication.
    fn retry_machine_from_navigator(
        &mut self, name: String, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if !crate::features::terminal::herdr_enabled()
            || !self.machine_snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.machines.iter().any(|machine| machine.name == name)
            })
        {
            window.push_notification(Notification::info(
                "This machine is not registered with the active Herdr Core.",
            ), cx);
            return;
        }
        let task = cx.background_executor().spawn({
            let name = name.clone();
            async move { machine_model::reconnect_machine(&name) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(machine) => {
                        if let Some(snapshot) = &mut this.machine_snapshot
                            && let Some(saved) = snapshot.machines.iter_mut()
                                .find(|saved| saved.name == machine.name)
                        {
                            *saved = machine.clone();
                        }
                        window.push_notification(Notification::info(format!(
                            "Herdr {}: {}. Its automatic reconnect remains enabled.",
                            machine.name, navigator_machine_status(&machine.state),
                        )), cx);
                    }
                    Err(error) => window.push_notification(
                        Notification::error(format!(
                            "Could not reconnect {name} through Herdr: {error}",
                        )), cx,
                    ),
                }
                cx.notify();
            });
        }).detach();
    }

    /// The same session-selection behavior backs both the full navigator and
    /// the 48px rail. A click is navigation, never an implicit new agent.
    fn select_navigator_session(
        &mut self,
        source: &NavigatorSessionSource,
        rename_on_double_click: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match source {
            NavigatorSessionSource::Open { space, tab, pane } => {
                if *space != self.current {
                    self.switch_workspace(*space, window, cx);
                }
                self.activate(*tab, window, cx);
                self.focus_pane(*pane, window, cx);
                if rename_on_double_click {
                    self.start_tab_rename(*tab, window, cx);
                }
            }
            NavigatorSessionSource::Saved { space, tab, .. } => {
                self.switch_workspace(*space, window, cx);
                self.activate(*tab, window, cx);
            }
            NavigatorSessionSource::Herdr { address, live, agent } => {
                if *live {
                    self.show_or_attach_herdr_session(
                        address.clone(), *agent, window, cx,
                    );
                } else {
                    window.push_notification(Notification::info(
                        "This Herdr session is not live. No new agent was created.",
                    ), cx);
                }
            }
        }
    }

    /// The narrow rail is a fully functional session switcher, not a list
    /// of computers. Each computer is a small visual group header and every
    /// agent/terminal beneath it is directly selectable without expanding.
    pub(super) fn render_machine_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = self.navigator_groups();
        let active_machine = self.active_navigator_machine_key();
        let herdr = crate::features::terminal::herdr_enabled();
        v_flex()
            .size_full()
            .min_h_0()
            .items_center()
            .child(
                div()
                    .w_full()
                    .h(px(42.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(ui::border())
                    .child(
                        Button::new("machine-rail-expand")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::PanelLeftOpen))
                            .tooltip("Expand Machines (Ctrl+B)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_sidebar(cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("machine-rail-scroll")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py_1()
                    .child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .children(groups.into_iter().enumerate().map(|(group_index, group)| {
                                let group_active = group.key == active_machine;
                                let key = group.key.clone();
                                let icon = if group.local {
                                    IconName::SquareTerminal
                                } else {
                                    IconName::Layers
                                };
                                let state = navigator_machine_status(&group.state);
                                let indicator = match group.state.as_str() {
                                    "connected" => ui::accent(),
                                    "connecting" => ui::petal(),
                                    "needs_login" | "needs_update" | "unreachable" => ui::warning(),
                                    _ => ui::muted(),
                                };
                                let name = group.name.clone();
                                let count = group.sessions.len();
                                let header_tip = format!(
                                    "{name} · {state} · {count} session{} · Expand machine",
                                    if count == 1 { "" } else { "s" },
                                );
                                v_flex()
                                    .id(("machine-rail-group", group_index))
                                    .w_full()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .id(("machine-rail-header", group_index))
                                            .w_full()
                                            .h(px(29.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .rounded(px(5.))
                                            .when(group_active, |row| {
                                                row.bg(ui::surface_hover().opacity(0.35))
                                            })
                                            .hover(|row| row.bg(ui::surface_hover().opacity(0.55)))
                                            .tooltip(move |window, cx| {
                                                Tooltip::new(header_tip.clone()).build(window, cx)
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.expand_machine_from_rail(key.clone(), cx);
                                            }))
                                            .child(
                                                h_flex()
                                                    .gap(px(3.))
                                                    .items_center()
                                                    .child(
                                                        Icon::new(icon)
                                                            .xsmall()
                                                            .text_color(if group_active {
                                                                ui::petal()
                                                            } else {
                                                                ui::muted()
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .size(px(5.))
                                                            .rounded_full()
                                                            .bg(indicator),
                                                    ),
                                            ),
                                    )
                                    // Session rows remain visible even if the
                                    // corresponding full-width group is closed.
                                    .children(group.sessions.into_iter().map(
                                        |session| {
                                            let source = session.source.clone();
                                            let active = matches!(
                                                &source,
                                                NavigatorSessionSource::Open { space, tab, pane }
                                                    if *space == self.current
                                                        && *tab == self.space().active
                                                        && self.tab().is_some_and(|t| t.focused == *pane)
                                            );
                                            let title = session.title.clone();
                                            let kind = session.kind;
                                            let running = session.running;
                                            let directory = session.directory.clone()
                                                .unwrap_or_else(|| "cwd unavailable".into());
                                            let mut details = format!(
                                                "{name} · {kind}: {title} · {directory}",
                                            );
                                            if let Some(address) = &session.address {
                                                details.push_str(&format!(" · {address}"));
                                            }
                                            let icon = session.icon;
                                            div()
                                                .id(SharedString::from(format!(
                                                    "machine-rail-session-{}", session.row_id,
                                                )))
                                                .w_full()
                                                .h(px(39.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .cursor_pointer()
                                                .rounded(px(6.))
                                                .when(active, |row| {
                                                    row.bg(ui::surface_hover().opacity(0.9))
                                                })
                                                .hover(|row| {
                                                    row.bg(ui::surface_hover().opacity(0.5))
                                                })
                                                .tooltip(move |window, cx| {
                                                    Tooltip::new(details.clone()).build(window, cx)
                                                })
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    this.select_navigator_session(
                                                        &source, false, window, cx,
                                                    );
                                                }))
                                                .child(
                                                    v_flex()
                                                        .items_center()
                                                        .gap(px(3.))
                                                        .child(
                                                            Icon::new(icon)
                                                                .small()
                                                                .text_color(if active {
                                                                    ui::petal()
                                                                } else if running {
                                                                    ui::accent()
                                                                } else {
                                                                    ui::muted()
                                                                }),
                                                        )
                                                        .child(
                                                            div()
                                                                .size(px(5.))
                                                                .rounded_full()
                                                                .bg(if running {
                                                                    ui::accent()
                                                                } else {
                                                                    ui::muted()
                                                                }),
                                                        ),
                                                )
                                        }
                                    ))
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(38.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_t_1()
                    .border_color(ui::border())
                    .when(herdr, |bar| {
                        bar.child(
                            Button::new("machine-rail-manage")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::Settings))
                                .tooltip("Manage Herdr machines")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_machines(window, cx);
                                })),
                        )
                    }),
            )
    }

    pub(super) fn render_machine_navigator(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = self.navigator_groups();
        let active_machine = self.active_navigator_machine_key();
        let herdr = crate::features::terminal::herdr_enabled();
        let total: usize = groups.iter().map(|g| g.sessions.len()).sum();
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(42.))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(ui::border())
                    .child(
                        Icon::new(IconName::Layers)
                            .small()
                            .text_color(ui::petal()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Machines"),
                    )
                    .child(
                        Button::new("navigator-add-machine")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(if herdr {
                                IconName::Plus
                            } else {
                                IconName::Bot
                            }))
                            .tooltip(if herdr {
                                "Add a machine through Herdr Core"
                            } else {
                                "New local agent"
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if herdr {
                                    this.open_machine_registration(window, cx);
                                } else {
                                    this.new_agent_in_group(None, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("navigator-collapse")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::PanelLeftClose))
                            .tooltip("Collapse Machines to the slim rail (Ctrl+B)")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    )
                    .when(herdr, |row| {
                        row.child(
                            Button::new("navigator-machine-manage")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::Settings))
                                .tooltip("Manage registered Herdr machines")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_machines(window, cx)
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .id("machine-agent-navigator-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py_2()
                    .child(
                        v_flex()
                            .gap_1()
                            .children(groups.into_iter().enumerate().map(|(group_index, group)| {
                                let collapsed = self.collapsed_machines.contains(&group.key);
                                let key = group.key.clone();
                                let local = group.local;
                                let available = group.connected;
                                let active = group.key == active_machine;
                                let state = navigator_machine_status(&group.state);
                                let state_color = match group.state.as_str() {
                                    "connected" => ui::accent(),
                                    "connecting" => ui::petal(),
                                    "needs_login" | "needs_update" | "unreachable" => ui::warning(),
                                    _ => ui::muted(),
                                };
                                let group_tooltip = format!(
                                    "{} · {} · {} sessions", group.name, state, group.sessions.len(),
                                );
                                let machine_for_add = if local { None } else {
                                    Some(group.key.clone())
                                };
                                let machine_for_actions = group.key.clone();
                                v_flex()
                                    .id(("navigator-machine", group_index))
                                    .gap(px(1.))
                                    .child(
                                        h_flex()
                                            .id(("navigator-machine-toggle", group_index))
                                            .h(px(34.))
                                            .px_2()
                                            .gap_2()
                                            .cursor_pointer()
                                            .when(active, |row| row.bg(ui::surface_hover().opacity(0.8)))
                                            .hover(|row| row.bg(ui::surface_hover().opacity(0.5)))
                                            .tooltip(move |window, cx| {
                                                Tooltip::new(group_tooltip.clone()).build(window, cx)
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle_machine_group(key.clone(), cx)
                                            }))
                                            .child(
                                                Icon::new(if collapsed {
                                                    IconName::ChevronRight
                                                } else {
                                                    IconName::ChevronDown
                                                })
                                                    .xsmall()
                                                    .text_color(ui::muted()),
                                            )
                                            .child(
                                                Icon::new(if local {
                                                    IconName::SquareTerminal
                                                } else {
                                                    IconName::Layers
                                                })
                                                    .small()
                                                    .text_color(ui::muted()),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_xs()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(group.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(state_color),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(ui::muted())
                                                    .child(group.sessions.len().to_string()),
                                            )
                                            .when(herdr && !local, |row| {
                                                let weak = cx.entity().downgrade();
                                                row.child(
                                                    div()
                                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                            cx.stop_propagation();
                                                        })
                                                        .child(
                                                            Button::new(("navigator-machine-actions", group_index))
                                                                .ghost()
                                                                .xsmall()
                                                                .icon(Icon::new(IconName::Ellipsis))
                                                                .tooltip("Machine actions")
                                                                .dropdown_menu(move |menu, _, _| {
                                                                    let menu = menu.min_w(px(220.));
                                                                    let menu = if available {
                                                                        menu
                                                                    } else {
                                                                        let weak_retry = weak.clone();
                                                                        let name = machine_for_actions.clone();
                                                                        menu.item(
                                                                            PopupMenuItem::element(|_, _| {
                                                                                h_flex().gap_2().px_2().py_1p5()
                                                                                    .child(Icon::new(IconName::RefreshCw).small())
                                                                                    .child("Retry connection")
                                                                            })
                                                                            .on_click(move |_, window, cx| {
                                                                                let _ = weak_retry.update(cx, |this, cx| {
                                                                                    this.retry_machine_from_navigator(
                                                                                        name.clone(), window, cx,
                                                                                    );
                                                                                });
                                                                            }),
                                                                        )
                                                                    };
                                                                    let weak_manage = weak.clone();
                                                                    menu.item(
                                                                        PopupMenuItem::element(|_, _| {
                                                                            h_flex().gap_2().px_2().py_1p5()
                                                                                .child(Icon::new(IconName::Settings).small())
                                                                                .child("Manage machines")
                                                                        })
                                                                        .on_click(move |_, window, cx| {
                                                                            let _ = weak_manage.update(cx, |this, cx| {
                                                                                this.open_machines(window, cx);
                                                                            });
                                                                        }),
                                                                    )
                                                                }),
                                                        ),
                                                )
                                            })
                                            .child({
                                                // Do not bubble the + click into the
                                                // machine row's expand/collapse handler.
                                                let weak = cx.entity().downgrade();
                                                div()
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation();
                                                    })
                                                    .child(
                                                        Button::new(("navigator-new-session", group_index))
                                                            .ghost()
                                                            .xsmall()
                                                            .icon(Icon::new(IconName::Plus))
                                                            .tooltip(if available {
                                                                "New Gentle Shell or Terminal"
                                                            } else {
                                                                "Machine disconnected"
                                                            })
                                                            .disabled(!available)
                                                            .dropdown_menu(move |menu, _, _| {
                                                                    let weak_agent = weak.clone();
                                                                    let weak_terminal = weak.clone();
                                                                    let agent_machine = machine_for_add.clone();
                                                                    let terminal_machine = machine_for_add.clone();
                                                                    menu.min_w(px(210.))
                                                                        .item(
                                                                            PopupMenuItem::element(|_, _| {
                                                                                h_flex().gap_2().px_2().py_1p5()
                                                                                    .child(Icon::new(IconName::Bot).small())
                                                                                    .child("Gentle Shell")
                                                                            })
                                                                            .on_click(move |_, window, cx| {
                                                                                let _ = weak_agent.update(cx, |this, cx| {
                                                                                    this.new_agent_in_group(
                                                                                        agent_machine.clone(), window, cx,
                                                                                    );
                                                                                });
                                                                            }),
                                                                        )
                                                                        .item(
                                                                            PopupMenuItem::element(|_, _| {
                                                                                h_flex().gap_2().px_2().py_1p5()
                                                                                    .child(Icon::new(IconName::SquareTerminal).small())
                                                                                    .child("Terminal")
                                                                            })
                                                                            .on_click(move |_, window, cx| {
                                                                                let _ = weak_terminal.update(cx, |this, cx| {
                                                                                    this.new_terminal_in_group(
                                                                                        terminal_machine.clone(), window, cx,
                                                                                    );
                                                                                });
                                                                            }),
                                                                        )
                                                                }),
                                                    )
                                            }),
                                    )
                                    .when(!collapsed, |section| {
                                        section.child(
                                            v_flex()
                                                .gap(px(1.))
                                                .children(group.sessions.into_iter().map(|session| {
                                                    let source = session.source.clone();
                                                    let active = matches!(&source,
                                                        NavigatorSessionSource::Open { space, tab, pane, .. }
                                                            if *space == self.current
                                                                && *tab == self.space().active
                                                                && self.tab().is_some_and(|t| t.focused == *pane)
                                                    );
                                                    let renaming = match &source {
                                                        NavigatorSessionSource::Open { space, pane, .. }
                                                            if *space == self.current =>
                                                                self.renaming.as_ref()
                                                                    .filter(|rename| rename.pane == *pane),
                                                        _ => None,
                                                    };
                                                    let group_name = format!("nav-session-{}", session.row_id);
                                                    let close_id = format!("nav-close-{}", session.row_id);
                                                    let close_source = session.source.clone();
                                                    let close_address = session.address.clone();
                                                    let close_agent = session.kind == "Gentle Shell";
                                                    let details = format!(
                                                        "{} · {}{}{}",
                                                        session.kind, session.title,
                                                        session.address.as_ref()
                                                            .map(|a| format!(" · {a}"))
                                                            .unwrap_or_default(),
                                                        if session.pane_count > 1 {
                                                            format!(" · {} panes", session.pane_count)
                                                        } else { String::new() },
                                                    );
                                                    let details = match &session.directory {
                                                        Some(path) => format!("{details} · cwd: {path}"),
                                                        None => details,
                                                    };
                                                    let directory_label = format!(
                                                        "{} · {}",
                                                        session.kind,
                                                        session.directory.as_deref()
                                                            .unwrap_or("cwd unavailable"),
                                                    );
                                                    h_flex()
                                                        .id(SharedString::from(format!("nav-row-{}", session.row_id)))
                                                        .group(SharedString::from(group_name.clone()))
                                                        .h(px(48.))
                                                        .ml(px(14.))
                                                        .mr(px(5.))
                                                        .pl(px(10.))
                                                        .pr(px(5.))
                                                        .gap_2()
                                                        .rounded(px(6.))
                                                        .cursor_pointer()
                                                        .when(active, |row| {
                                                            row.bg(ui::surface_hover().opacity(0.85))
                                                               .text_color(ui::fg())
                                                        })
                                                        .when(!active, |row| {
                                                            row.text_color(ui::muted())
                                                               .hover(|row| row.bg(ui::surface_hover().opacity(0.45)))
                                                        })
                                                        .tooltip(move |window, cx| {
                                                            Tooltip::new(details.clone()).build(window, cx)
                                                        })
                                                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                                            this.select_navigator_session(
                                                                &source, event.click_count() >= 2, window, cx,
                                                            );
                                                        }))
                                                        .child(
                                                            Icon::new(session.icon)
                                                                .small()
                                                                .text_color(if active {
                                                                    ui::petal()
                                                                } else if session.running {
                                                                    ui::accent()
                                                                } else {
                                                                    ui::muted()
                                                                }),
                                                        )
                                                        .child(match renaming {
                                                            Some(rename) => div()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .h(px(24.))
                                                                .px_1()
                                                                .flex()
                                                                .items_center()
                                                                .rounded(px(5.))
                                                                .bg(ui::well())
                                                                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                                    cx.stop_propagation();
                                                                })
                                                                .on_action(cx.listener(Self::cancel_tab_rename))
                                                                .child(Input::new(&rename.input).appearance(false).xsmall())
                                                                .into_any_element(),
                                                            None => v_flex()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .gap(px(2.))
                                                                .child(
                                                                    div()
                                                                        .min_w_0()
                                                                        .overflow_hidden()
                                                                        .text_ellipsis()
                                                                        .whitespace_nowrap()
                                                                        .text_xs()
                                                                        .font_weight(if active {
                                                                            FontWeight::SEMIBOLD
                                                                        } else {
                                                                            FontWeight::NORMAL
                                                                        })
                                                                        .child(session.title),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .min_w_0()
                                                                        .overflow_hidden()
                                                                        .text_ellipsis()
                                                                        .whitespace_nowrap()
                                                                        .text_size(px(10.))
                                                                        .text_color(ui::faint())
                                                                        .child(directory_label),
                                                                )
                                                                .into_any_element(),
                                                        })
                                                        .when(session.running, |row| {
                                                            row.child(
                                                                div().size(px(6.)).rounded_full()
                                                                    .bg(ui::accent()),
                                                            )
                                                        })
                                                        .child(
                                                            div()
                                                                .id(SharedString::from(close_id))
                                                                .size(px(20.))
                                                                .flex_none()
                                                                .flex()
                                                                .items_center()
                                                                .justify_center()
                                                                .rounded(px(4.))
                                                                .text_color(ui::muted())
                                                                .invisible()
                                                                .group_hover(
                                                                    SharedString::from(group_name),
                                                                    |button| button.visible(),
                                                                )
                                                                .hover(|button| {
                                                                    button.bg(ui::surface_hover())
                                                                        .text_color(ui::danger())
                                                                })
                                                                .tooltip(|window, cx| {
                                                                    Tooltip::new("Terminate session permanently")
                                                                        .build(window, cx)
                                                                })
                                                                .on_click(cx.listener(
                                                                    move |this, _, window, cx| {
                                                                        cx.stop_propagation();
                                                                        this.confirm_permanent_session_close(
                                                                            close_source.clone(),
                                                                            close_address.clone(),
                                                                            close_agent,
                                                                            window, cx,
                                                                        );
                                                                    },
                                                                ))
                                                                .child(Icon::new(IconName::X).xsmall()),
                                                        )
                                                })),
                                        )
                                    })
                            })),
                    ),
            )
            .when_some(self.machine_error.clone().filter(|_| herdr), |panel, error| {
                panel.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(ui::danger())
                        .child(format!("Herdr: {error}")),
                )
            })
            .child(
                h_flex()
                    .flex_none()
                    .h(px(30.))
                    .px_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(ui::border())
                    .text_xs()
                    .text_color(ui::muted())
                    .child(format!("{total} sessions"))
                    .child(div().flex_1())
                    .child(
                        Button::new("navigator-files")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Folder))
                            .tooltip("Toggle local Windows Files panel on the right")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_sidebar_mode(cx)
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{machine_key, navigator_machine_status};
    use pi_rpc::AgentTarget;

    #[test]
    fn distinguishes_core_connection_states_without_claiming_saved_machine_is_online() {
        assert_eq!(navigator_machine_status("connected"), "Connected");
        assert_eq!(navigator_machine_status("connecting"), "Connecting");
        assert_eq!(navigator_machine_status("needs_login"), "Needs login");
        assert_eq!(navigator_machine_status("needs_update"), "Needs update");
        assert_eq!(navigator_machine_status("unreachable"), "Unreachable");
        assert_eq!(navigator_machine_status("unknown"), "Status unknown");
    }

    #[test]
    fn distinguishes_local_and_remote_runtime_directories() {
        use super::session_directory;
        use std::path::Path;
        let local = AgentTarget::Local;
        let remote = AgentTarget::Remote {
            machine: "javipi".into(),
            cwd: "/srv/workspaces/JaviPi".into(),
        };
        let windows = Path::new(r"D:\\Programacion\\gentle-shell-gpui-herdr-core");
        assert_eq!(
            session_directory(Some(&remote), None, windows, None).as_deref(),
            Some("/srv/workspaces/JaviPi"),
        );
        assert_eq!(
            session_directory(Some(&local), Some("javipi/t2"), windows, None),
            Some(windows.display().to_string()),
        );
        assert_eq!(session_directory(None, Some("javipi/t3"), windows, None), None);
    }

    #[test]
    fn recovered_remote_directory_comes_from_herdr_not_windows() {
        use super::session_directory;
        use crate::features::machines::model::{Snapshot, Terminal};
        use std::path::Path;
        let snapshot = Snapshot {
            local_name: "local".into(),
            machines: Vec::new(),
            terminals: vec![Terminal {
                address: "javipi/t42".into(),
                title: "Gentle Shell".into(),
                cwd: "/srv/workspaces/OtherProject".into(),
                state: "live".into(),
                gentle_shell: true,
            }],
        };
        assert_eq!(
            session_directory(
                None, Some("javipi/t42"), Path::new("C:/wrong"),
                Some(&snapshot),
            ).as_deref(),
            Some("/srv/workspaces/OtherProject"),
        );
    }

    #[test]
    fn target_precedes_stale_address() {
        let local = AgentTarget::Local;
        let remote = AgentTarget::Remote {
            machine: "javipi".into(), cwd: "/srv/workspaces/JaviPi".into(),
        };
        assert_eq!(machine_key(Some(&local), Some("javipi/t22")), None);
        assert_eq!(machine_key(Some(&remote), Some("other/t1")), Some("javipi".into()));
        assert_eq!(machine_key(None, Some("javipi/t33")), Some("javipi".into()));
        assert_eq!(machine_key(None, Some("t9")), None);
    }
}
