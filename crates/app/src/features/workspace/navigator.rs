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
    local: bool,
    sessions: Vec<NavigatorSession>,
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
    /// Machine and runtime path for the selected agent, shown separately
    /// from the Windows workspace name in the title bar.
    pub(super) fn active_runtime_label(&self) -> String {
        let Some(tab) = self.tab() else {
            return "No agent selected".into();
        };
        let primary = tab.tree.leaves().into_iter()
            .find(|id| self.panes.get(id).is_some_and(|pane| {
                pane.item.kind == PaneKind::Agent
            }))
            .unwrap_or(tab.focused);
        let Some(pane) = self.panes.get(&primary) else {
            return "No agent selected".into();
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
                        local: false,
                        sessions: Vec::new(),
                    });
                }
            }
        }

        let mut shown = std::collections::HashSet::new();
        let mut add = |key: String, entry: NavigatorSession| {
            if let Some(address) = &entry.address
                && !shown.insert(address.clone())
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

    /// Known agent identities are shared with the standalone Machines
    /// manager so its terminal list can label those entries correctly.
    pub(super) fn known_herdr_agent_addresses(&self, cx: &App)
        -> std::collections::HashSet<String>
    {
        let mut addresses = std::collections::HashSet::new();
        for pane in self.panes.values() {
            if pane.item.kind != PaneKind::Agent { continue; }
            if let Some(address) = pane.herdr_address.clone().or_else(|| {
                pane.item.terminal_address.as_ref().and_then(|get| get(cx))
            }) {
                addresses.insert(address);
            }
        }
        for space in &self.spaces {
            if let Some((tabs, _)) = &space.saved {
                for tab in tabs {
                    for pane in tab.layout.leaves() {
                        if pane.kind == PaneKind::Agent
                            && let Some(address) = &pane.herdr_terminal
                        {
                            addresses.insert(address.clone());
                        }
                    }
                }
            }
        }
        addresses
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

    pub(super) fn render_machine_navigator(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = self.navigator_groups();
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
                                "Add SSH machine to Herdr Core"
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
                    .when(herdr, |row| {
                        row.child(
                            Button::new("navigator-machine-manage")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::Settings))
                                .tooltip("Manage Herdr machines and terminals")
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
                                let machine_for_add = if local { None } else {
                                    Some(group.key.clone())
                                };
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
                                            .hover(|row| row.bg(ui::surface_hover().opacity(0.5)))
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
                                                    .bg(if available {
                                                        ui::accent()
                                                    } else {
                                                        ui::muted()
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(ui::muted())
                                                    .child(group.sessions.len().to_string()),
                                            )
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
                                                    let close_target = match &source {
                                                        NavigatorSessionSource::Open { space, tab, .. }
                                                            if *space == self.current => Some(*tab),
                                                        _ => None,
                                                    };
                                                    let renaming = match &source {
                                                        NavigatorSessionSource::Open { space, pane, .. }
                                                            if *space == self.current =>
                                                                self.renaming.as_ref()
                                                                    .filter(|rename| rename.pane == *pane),
                                                        _ => None,
                                                    };
                                                    let group_name = format!("nav-session-{}", session.row_id);
                                                    let close_id = format!("nav-close-{}", session.row_id);
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
                                                            match &source {
                                                                NavigatorSessionSource::Open { space, tab, pane, .. } => {
                                                                    if *space != this.current {
                                                                        this.switch_workspace(*space, window, cx);
                                                                    }
                                                                    this.activate(*tab, window, cx);
                                                                    this.focus_pane(*pane, window, cx);
                                                                    if event.click_count() >= 2 {
                                                                        this.start_tab_rename(*tab, window, cx);
                                                                    }
                                                                }
                                                                NavigatorSessionSource::Saved { space, tab, .. } => {
                                                                    this.switch_workspace(*space, window, cx);
                                                                    this.activate(*tab, window, cx);
                                                                }
                                                                NavigatorSessionSource::Herdr { address, live, agent } => {
                                                                    if *live {
                                                                        this.show_or_attach_herdr_session(
                                                                            address.clone(), *agent, window, cx,
                                                                        );
                                                                    }
                                                                }
                                                            }
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
                                                        .when_some(close_target, |row, close_index| {
                                                            row.child(
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
                                                                            .text_color(ui::fg())
                                                                    })
                                                                    .on_click(cx.listener(
                                                                        move |this, _, window, cx| {
                                                                            cx.stop_propagation();
                                                                            this.close_tab(close_index, window, cx);
                                                                        },
                                                                    ))
                                                                    .child(Icon::new(IconName::X).xsmall()),
                                                            )
                                                        })
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
    use super::machine_key;
    use pi_rpc::AgentTarget;

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
