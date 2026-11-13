//! GPUI overlay for Herdr's local and remote machines.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::shared::ui;

use super::model::{Machine, Snapshot, Terminal};
use super::{MachinesEvent, MachinesView};

impl MachinesView {
    fn render_terminal(
        &self,
        terminal: &Terminal,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = terminal.live();
        let address = terminal.address.clone();
        let title = terminal.display_title().to_string();
        let subtitle = if terminal.cwd.is_empty() {
            terminal.address.clone()
        } else {
            format!("{} · {}", terminal.address, terminal.cwd)
        };
        h_flex()
            .id(("machines-terminal", index))
            .min_w_0()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_lg()
            .hover(|d| d.bg(ui::surface_hover()))
            .child(
                Icon::new(IconName::SquareTerminal)
                    .small()
                    .text_color(if active { ui::accent() } else { ui::faint() }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().truncate().text_sm().text_color(ui::fg()).child(title))
                    .child(div().truncate().text_xs().text_color(ui::muted()).child(subtitle)),
            )
            .child(ui::chip(
                if active { "live" } else { "finished" },
                if active { ui::accent() } else { ui::muted() },
            ))
            .when(active, |row| {
                row.child(
                    Button::new(("machines-open-terminal", index))
                        .ghost()
                        .xsmall()
                        .label("Open")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_terminal(address.clone(), cx)
                        })),
                )
            })
            .into_any_element()
    }

    fn render_group(
        &self,
        snapshot: &Snapshot,
        machine: Option<&Machine>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = machine.map(|m| m.name.as_str());
        let local = machine.is_none();
        let connected = machine.is_none_or(Machine::connected);
        let title = machine.map_or(snapshot.local_name.as_str(), |m| m.name.as_str());
        let state = machine.map_or("local", |m| m.state.as_str());
        let state_color = if connected { ui::accent() } else { ui::warning() };
        let host = machine.map_or("This computer".to_string(), |m| {
            if m.host.trim().is_empty() {
                m.target.clone()
            } else {
                format!("{} · {}", m.host, m.target)
            }
        });
        let terminals: Vec<&Terminal> = snapshot
            .terminals
            .iter()
            .filter(|terminal| terminal.belongs_to(name))
            .collect();
        let count = terminals.len();
        let live = terminals.iter().filter(|t| t.live()).count();
        let action = machine.map(|m| m.name.clone());
        let rows: Vec<AnyElement> = terminals
            .into_iter()
            .take(150)
            .enumerate()
            .map(|(i, terminal)| self.render_terminal(terminal, index * 1000 + i, cx))
            .collect();

        let toolbar = h_flex()
            .min_w_0()
            .gap_2()
            .child(
                Icon::new(IconName::SquareTerminal)
                    .small()
                    .text_color(ui::muted()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui::fg())
                    .child(title.to_string()),
            )
            .child(ui::chip(state.to_string(), state_color).flex_shrink_0())
            .child(div().flex_1())
            .when(connected, |bar| {
                let machine_for_agent = action.clone();
                bar.child(
                    Button::new(("machines-new-agent", index))
                        .small()
                        .ghost()
                        .icon(Icon::new(IconName::Bot))
                        .label("New agent")
                        .loading(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.new_agent(machine_for_agent.clone(), cx)
                        })),
                )
            })
            .child(
                if connected {
                    Button::new(("machines-new-shell", index))
                        .small()
                        .ghost()
                        .icon(Icon::new(IconName::Plus))
                        .label("New shell")
                        .loading(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.new_shell(action.clone(), cx)
                        }))
                } else {
                    let name = machine.expect("non-local machine").name.clone();
                    Button::new(("machines-connect", index))
                        .small()
                        .ghost()
                        .label("Connect")
                        .loading(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.connect_machine(name.clone(), cx)
                        }))
                },
            );

        v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(ui::border())
            .rounded_lg()
            .bg(ui::panel())
            .child(toolbar)
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(ui::muted())
                    .child(format!(
                        "{host} · {live} active of {count} terminal{}",
                        if count == 1 { "" } else { "s" },
                    )),
            )
            .when_some(machine.and_then(|m| m.why.as_deref()), |d, why| {
                d.child(div().text_xs().text_color(ui::warning()).child(why.to_string()))
            })
            .when(rows.is_empty(), |d| {
                d.child(
                    div()
                        .py_3()
                        .px_2()
                        .text_xs()
                        .text_color(ui::faint())
                        .child(if !connected {
                            "No terminal list while this machine is disconnected."
                        } else if local {
                            "No local Herdr terminals. Use New shell to create one."
                        } else {
                            "No active terminals on this machine."
                        }),
                )
            })
            .children(rows)
            .when(count > 150, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(ui::muted())
                        .child(format!("Showing 150 of {count} terminals.")),
                )
            })
            .into_any_element()
    }

    fn render_groups(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut groups = Vec::with_capacity(snapshot.machines.len() + 1);
        groups.push(self.render_group(snapshot, None, 0, cx));
        for (index, machine) in snapshot.machines.iter().enumerate() {
            groups.push(self.render_group(snapshot, Some(machine), index + 1, cx));
        }
        groups
    }
}

impl Render for MachinesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.snapshot.as_ref().map_or(0, |s| s.machines.len());
        let groups = self.render_groups(cx);
        let (backdrop, panel) = ui::screen(Some(820.));
        backdrop.occlude().child(ui::pop_in(
            "machines-screen",
            panel
                .w_full()
                .max_w(px(820.))
                .child(
                    ui::screen_header()
                        .child(
                            Icon::new(IconName::SquareTerminal)
                                .small()
                                .text_color(ui::muted()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ui::fg())
                                .child("Machines"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(ui::muted())
                                .child("Herdr Core · local and remote terminals"),
                        )
                        .child(ui::chip(format!("{count} remote"), ui::muted()))
                        .child(
                            Button::new("machines-close")
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(IconName::X))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(MachinesEvent::Close))),
                        ),
                )
                .child(
                    h_flex()
                        .px_4()
                        .pt_4()
                        .pb_2()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(ui::muted())
                                .child(if self.loading {
                                    "Refreshing Herdr machines and terminals..."
                                } else {
                                    "Open an existing session or start a new persistent shell."
                                }),
                        )
                        .child(
                            Button::new("machines-refresh")
                                .ghost()
                                .small()
                                .icon(Icon::new(IconName::RefreshCw))
                                .label("Refresh")
                                .loading(self.loading)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                )
                .when_some(self.info.clone(), |panel, info| {
                    panel.child(
                        div()
                            .mx_4()
                            .mb_2()
                            .text_xs()
                            .text_color(ui::muted())
                            .child(info),
                    )
                })
                .when_some(self.error.clone(), |panel, error| {
                    panel.child(
                        div()
                            .mx_4()
                            .mb_2()
                            .p_3()
                            .rounded_lg()
                            .bg(ui::well())
                            .text_xs()
                            .text_color(ui::danger())
                            .child(format!("Herdr: {error}")),
                    )
                })
                .child(
                    div()
                        .id("machines-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_4()
                        .pb_4()
                        .child(
                            v_flex()
                                .gap_3()
                                .when(groups.is_empty() && self.loading, |d| {
                                    d.child(
                                        div()
                                            .py_6()
                                            .text_sm()
                                            .text_color(ui::muted())
                                            .child("Discovering machines..."),
                                    )
                                })
                                .children(groups),
                        ),
                ),
        ))
    }
}
