//! Machines management modal: registered computers only.
//! Never show, open, close or create individual sessions from this screen.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::shared::ui;

use super::model::Machine;
use super::{MachinesEvent, MachinesView};

impl MachinesView {
    fn render_machine(
        &self, machine: &Machine, index: usize, cx: &mut Context<Self>,
    ) -> AnyElement {
        let connected = machine.connected();
        let name = machine.name.clone();
        let subtitle = if !machine.host.trim().is_empty() {
            format!("{} · {}", machine.host, machine.target)
        } else if machine.target.trim().is_empty() {
            "Registered in Herdr Core".into()
        } else {
            machine.target.clone()
        };
        h_flex()
            .id(("machines-registered", index))
            .min_w_0()
            .gap_3()
            .px_3()
            .py_3()
            .border_1()
            .border_color(ui::border())
            .rounded_lg()
            .bg(ui::panel())
            .child(
                Icon::new(IconName::Layers)
                    .small()
                    .text_color(if connected { ui::accent() } else { ui::muted() }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div().text_sm().font_weight(FontWeight::SEMIBOLD)
                                    .truncate().child(name.clone()),
                            )
                            .child(ui::chip(
                                if connected { "Connected" } else { "Disconnected" },
                                if connected { ui::accent() } else { ui::warning() },
                            )),
                    )
                    .child(
                        div().text_xs().text_color(ui::muted())
                            .truncate().child(subtitle),
                    )
                    .when_some(machine.why.clone(), |col, why| {
                        col.child(
                            div().text_xs().text_color(ui::warning()).child(why),
                        )
                    }),
            )
            .child(
                Button::new(("machines-remove", index))
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::X))
                    .label("Remove")
                    .disabled(self.busy)
                    .tooltip("Forget this machine in Herdr; leave its sessions running")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_remove_machine(name.clone(), window, cx);
                    })),
            )
            .into_any_element()
    }
}

impl Render for MachinesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let remote_count = self.snapshot.as_ref().map_or(0, |s| s.machines.len());
        let connected_count = self.snapshot.as_ref().map_or(0, |s| {
            s.machines.iter().filter(|machine| machine.connected()).count()
        });
        let local_name = self.snapshot.as_ref()
            .map_or("This computer".to_string(), |s| s.local_name.clone());
        let machines: Vec<AnyElement> = self.snapshot.as_ref()
            .map(|snapshot| {
                snapshot.machines.iter().enumerate()
                    .map(|(index, machine)| self.render_machine(machine, index, cx))
                    .collect()
            })
            .unwrap_or_default();
        let (backdrop, panel) = ui::screen(Some(740.));
        backdrop.occlude().child(ui::pop_in(
            "machines-screen",
            panel
                .w_full()
                .max_w(px(740.))
                .child(
                    ui::screen_header()
                        .child(
                            Icon::new(IconName::Layers)
                                .small().text_color(ui::petal()),
                        )
                        .child(
                            div().flex_shrink_0().text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Manage machines"),
                        )
                        .child(
                            div().flex_1().min_w_0().truncate()
                                .text_xs().text_color(ui::muted())
                                .child("Herdr Core · registered computers"),
                        )
                        .child(ui::chip(format!("{connected_count}/{remote_count} connected"), ui::muted()))
                        .child(
                            Button::new("machines-register")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::Plus))
                                .tooltip("Register another machine using Herdr")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.begin_add_machine(window, cx);
                                })),
                        )
                        .child(
                            Button::new("machines-close")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::X))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(MachinesEvent::Close))),
                        ),
                )
                .child(
                    h_flex()
                        .px_4().pt_4().pb_2().gap_2()
                        .child(
                            div().flex_1().text_xs().text_color(ui::muted())
                                .child("Manage machine connections. Agents and terminals stay in the sidebar."),
                        )
                        .child(
                            Button::new("machines-refresh")
                                .ghost().small()
                                .icon(Icon::new(IconName::RefreshCw))
                                .label("Refresh")
                                .loading(self.loading)
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                )
                .when(self.show_add_form, |panel| {
                    panel.child(
                        v_flex()
                            .mx_4().mb_3().p_3().gap_3()
                            .rounded_lg().border_1().border_color(ui::border())
                            .bg(ui::panel())
                            .child(
                                h_flex().gap_2()
                                    .child(Icon::new(IconName::Plus).small().text_color(ui::petal()))
                                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD)
                                        .child("Add machine to Herdr")),
                            )
                            .child(
                                div().text_xs().text_color(ui::muted())
                                    .child("Choose how Herdr reaches the other Core."),
                            )
                            .child(
                                h_flex().gap_2()
                                    .child(
                                        Button::new("machines-via")
                                            .small().label("Herdr connection command")
                                            .when(self.add_via, |button| button.primary())
                                            .when(!self.add_via, |button| button.ghost())
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_add_via(true, window, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("machines-host")
                                            .small().label("Host")
                                            .when(!self.add_via, |button| button.primary())
                                            .when(self.add_via, |button| button.ghost())
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_add_via(false, window, cx);
                                            })),
                                    ),
                            )
                            .child(
                                div().text_xs().text_color(ui::muted())
                                    .child("Machine name"),
                            )
                            .child(
                                ui::field()
                                    .child(Icon::new(IconName::Layers)
                                        .small().text_color(ui::faint()))
                                    .child(Input::new(&self.add_name).appearance(false).small()),
                            )
                            .child(
                                div().text_xs().text_color(ui::muted())
                                    .child(if self.add_via {
                                        "Command to reach the remote Herdr Core"
                                    } else {
                                        "Machine host or network address"
                                    }),
                            )
                            .when(self.add_via, |col| {
                                col.child(
                                    ui::field()
                                        .child(Icon::new(IconName::SquareTerminal)
                                            .small().text_color(ui::faint()))
                                        .child(Input::new(&self.add_command)
                                            .appearance(false).small()),
                                )
                            })
                            .when(!self.add_via, |col| {
                                col.child(
                                    ui::field()
                                        .child(Icon::new(IconName::Layers)
                                            .small().text_color(ui::faint()))
                                        .child(Input::new(&self.add_host)
                                            .appearance(false).small()),
                                )
                            })
                            .child(
                                div().text_xs().text_color(ui::muted())
                                    .child(if self.add_via {
                                        "Herdr runs its remote Core through this command, passing the remote command as the last argument. It must support bidirectional I/O without prompts (for example a configured WSL or provider CLI)."
                                    } else {
                                        "Herdr Core uses its host transport for this address (SSH by default). No agent or terminal is created."
                                    }),
                            )
                            .child(
                                h_flex().justify_end().gap_2()
                                    .child(
                                        Button::new("machines-add-cancel")
                                            .ghost().small().label("Cancel")
                                            .disabled(self.busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.cancel_add_machine(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("machines-add-confirm")
                                            .primary().small().label("Add machine")
                                            .loading(self.busy).disabled(self.busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.submit_add_machine(cx);
                                            })),
                                    ),
                            ),
                    )
                })
                .when_some(self.info.clone(), |panel, info| {
                    panel.child(div().mx_4().mb_2().text_xs()
                        .text_color(ui::muted()).child(info))
                })
                .when_some(self.error.clone(), |panel, error| {
                    panel.child(
                        div().mx_4().mb_2().p_3().rounded_lg().bg(ui::well())
                            .text_xs().text_color(ui::danger())
                            .child(format!("Herdr: {error}")),
                    )
                })
                .child(
                    div()
                        .id("machines-body")
                        .flex_1().min_h_0().overflow_y_scroll()
                        .px_4().pb_4()
                        .child(
                            v_flex().gap_2()
                                .child(
                                    h_flex().gap_2().px_3().py_3()
                                        .border_1().border_color(ui::border())
                                        .rounded_lg().bg(ui::panel())
                                        .child(Icon::new(IconName::SquareTerminal)
                                            .small().text_color(ui::accent()))
                                        .child(
                                            v_flex().flex_1().gap_1()
                                                .child(div().text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(local_name))
                                                .child(div().text_xs()
                                                    .text_color(ui::muted())
                                                    .child("This computer · local Herdr Core")),
                                        )
                                        .child(ui::chip("Local", ui::accent())),
                                )
                                .when(machines.is_empty(), |col| {
                                    col.child(
                                        div().px_3().py_5().text_xs()
                                            .text_color(ui::muted())
                                            .child(if self.loading {
                                                "Loading registered machines..."
                                            } else {
                                                "No remote machines registered. Use + to add one."
                                            }),
                                    )
                                })
                                .children(machines),
                        ),
                ),
        ))
    }
}
