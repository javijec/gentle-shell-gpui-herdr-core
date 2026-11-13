//! Machine-first navigation. Tabs remain as internal session containers for
//! existing persistence, split panes and Herdr remote agent ownership, but
//! there is deliberately no visual tab strip.

use super::*;

struct NavigatorSession {
    tab_index: usize,
    tab_id: usize,
    title: String,
    address: Option<String>,
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

impl Workspace {
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

        for (tab_index, tab) in self.space().tabs.iter().enumerate() {
            // A multi-pane tab follows its owning agent, not whichever file
            // pane currently happens to have keyboard focus.
            let primary = tab.tree.leaves().into_iter()
                .find(|id| self.panes.get(id).is_some_and(|pane| {
                    pane.item.kind == PaneKind::Agent
                }))
                .unwrap_or(tab.focused);
            let Some(pane) = self.panes.get(&primary) else { continue };
            let key = machine_key(
                pane.agent_target.as_ref(),
                pane.herdr_address.as_deref(),
            ).unwrap_or_else(|| "local".into());
            if !groups.iter().any(|group| group.key == key) {
                // Keep recovered sessions visible while the machine-list RPC
                // is temporarily unavailable. This does not assert online.
                groups.push(NavigatorMachine {
                    key: key.clone(), name: key.clone(),
                    connected: false, local: false, sessions: Vec::new(),
                });
            }
            if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
                group.sessions.push(NavigatorSession {
                    tab_index,
                    tab_id: tab.id,
                    title: {
                        let title = pane.title.to_string();
                        if title.is_empty() { "Agent".into() } else { title }
                    },
                    address: pane.herdr_address.clone(),
                    icon: pane.item.kind.icon(),
                    running: self.tab_running(tab),
                    pane_count: tab.tree.len(),
                });
            }
        }
        groups
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
        if crate::features::terminal::herdr_enabled() {
            let target = match machine {
                Some(machine) => pi_rpc::AgentTarget::remote_for_machine(machine),
                None => pi_rpc::AgentTarget::Local,
            };
            self.open_agent_on_machine(target, window, cx);
        } else if machine.is_none() {
            self.on_new_tab(&NewTab, window, cx);
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
                        Button::new("navigator-new-local")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Plus))
                            .tooltip("New agent on this computer")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_agent_in_group(None, window, cx)
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
                                            .child(
                                                Button::new(("navigator-add-machine", group_index))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(Icon::new(IconName::Plus))
                                                    .tooltip(if available {
                                                        "New agent on this machine"
                                                    } else {
                                                        "Machine disconnected"
                                                    })
                                                    .when(available, |button| {
                                                        button.on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                cx.stop_propagation();
                                                                this.new_agent_in_group(
                                                                    machine_for_add.clone(),
                                                                    window, cx,
                                                                )
                                                            },
                                                        ))
                                                    }),
                                            ),
                                    )
                                    .when(!collapsed, |section| {
                                        section.child(
                                            v_flex()
                                                .gap(px(1.))
                                                .children(group.sessions.into_iter().map(|session| {
                                                    let active = session.tab_index == self.space().active;
                                                    let tab_index = session.tab_index;
                                                    let close_index = tab_index;
                                                    let group_name = format!("nav-session-{}", session.tab_id);
                                                    let details = format!(
                                                        "{}{}{}",
                                                        session.title,
                                                        session.address.as_ref()
                                                            .map(|a| format!(" · {a}"))
                                                            .unwrap_or_default(),
                                                        if session.pane_count > 1 {
                                                            format!(" · {} panes", session.pane_count)
                                                        } else { String::new() },
                                                    );
                                                    h_flex()
                                                        .id(("navigator-session", session.tab_id))
                                                        .group(SharedString::from(group_name.clone()))
                                                        .h(px(34.))
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
                                                        .on_click(cx.listener(move |this, _, window, cx| {
                                                            this.activate(tab_index, window, cx)
                                                        }))
                                                        .child(
                                                            Icon::new(session.icon)
                                                                .small()
                                                                .text_color(if active {
                                                                    ui::petal()
                                                                } else {
                                                                    ui::muted()
                                                                }),
                                                        )
                                                        .child(
                                                            div()
                                                                .flex_1()
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
                                                        .when(session.running, |row| {
                                                            row.child(
                                                                div().size(px(6.)).rounded_full()
                                                                    .bg(ui::accent()),
                                                            )
                                                        })
                                                        .child(
                                                            div()
                                                                .id(("navigator-close", session.tab_id))
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
                            .tooltip("Show project files instead of machine navigator")
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
