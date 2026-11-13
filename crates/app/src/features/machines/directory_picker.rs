//! Choose a remote Linux working directory before creating a Pi agent.
//! Existing Herdr terminals provide useful shortcuts; a typed absolute
//! Linux path is also accepted. This is not a Windows folder picker.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::shared::ui;

use super::model::Snapshot;

/// This picker has no side effects until Launch is explicitly confirmed.
pub enum RemoteDirectoryEvent {
    Launch { machine: String, cwd: String },
    Close,
}

pub struct RemoteDirectoryPicker {
    machine: String,
    input: Entity<InputState>,
    suggestions: Vec<String>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RemoteDirectoryEvent> for RemoteDirectoryPicker {}

fn default_directory(machine: &str) -> String {
    match pi_rpc::AgentTarget::remote_for_machine(machine.to_owned()) {
        pi_rpc::AgentTarget::Remote { cwd, .. } => cwd,
        pi_rpc::AgentTarget::Local => unreachable!(),
    }
}

/// Shortcut paths are *known* from previous terminals, not a claim that
/// these are all filesystem directories. The target machine is mandatory.
fn directory_suggestions(machine: &str, snapshot: Option<&Snapshot>) -> Vec<String> {
    let mut paths = vec![default_directory(machine)];
    if machine.eq_ignore_ascii_case("javipi") {
        paths.push("/srv/workspaces".into());
    }
    if let Some(snapshot) = snapshot {
        let mut discovered: Vec<_> = snapshot.terminals.iter()
            .filter(|term| term.belongs_to(Some(machine)))
            .map(|term| term.cwd.trim().to_owned())
            .filter(|cwd| pi_rpc::AgentTarget::remote_with_cwd(
                machine.to_owned(), cwd.clone()
            ).is_ok())
            .collect();
        discovered.sort();
        for cwd in discovered {
            if !paths.contains(&cwd) {
                paths.push(cwd);
            }
        }
    }
    if !paths.iter().any(|path| path == "/") {
        paths.push("/".into());
    }
    paths.truncate(20);
    paths
}

impl RemoteDirectoryPicker {
    pub fn new(
        machine: String,
        snapshot: Option<Snapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(default_directory(&machine))
                .placeholder("Absolute Linux directory, e.g. /srv/workspaces/project")
        });
        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.launch(cx);
                } else {
                    this.error = None;
                    cx.notify();
                }
            },
        )];
        input.update(cx, |input, cx| input.focus(window, cx));
        Self {
            suggestions: directory_suggestions(&machine, snapshot.as_ref()),
            machine,
            input,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn launch(&mut self, cx: &mut Context<Self>) {
        let directory = self.input.read(cx).value().to_string();
        match pi_rpc::AgentTarget::remote_with_cwd(
            self.machine.clone(), directory,
        ) {
            Ok(pi_rpc::AgentTarget::Remote { machine, cwd }) => {
                cx.emit(RemoteDirectoryEvent::Launch { machine, cwd });
            }
            Ok(pi_rpc::AgentTarget::Local) => unreachable!(),
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn choose(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_value(path, window, cx);
            input.focus(window, cx);
        });
        self.error = None;
        cx.notify();
    }
}

impl Render for RemoteDirectoryPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let paths: Vec<AnyElement> = self.suggestions.iter().enumerate().map(|(i, path)| {
            let selected = self.input.read(cx).value() == path;
            let choice = path.clone();
            h_flex()
                .id(("remote-cwd-choice", i))
                .gap_2()
                .px_3()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |row| row.bg(ui::surface_hover()).text_color(ui::fg()))
                .when(!selected, |row| row.hover(|row| row.bg(ui::surface_hover().opacity(0.55))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose(choice.clone(), window, cx);
                }))
                .child(Icon::new(IconName::Folder).small().text_color(ui::muted()))
                .child(div().flex_1().min_w_0().truncate().text_sm().child(path.clone()))
                .into_any_element()
        }).collect();

        let (backdrop, panel) = ui::screen(Some(650.));
        backdrop.occlude().child(ui::pop_in(
            "remote-directory-picker",
            panel
                .w_full()
                .max_w(px(650.))
                .child(
                    ui::screen_header()
                        .child(Icon::new(IconName::FolderOpen).small().text_color(ui::petal()))
                        .child(div().text_base().font_weight(FontWeight::SEMIBOLD)
                            .child("New remote agent"))
                        .child(div().flex_1().min_w_0().truncate().text_xs()
                            .text_color(ui::muted())
                            .child(self.machine.clone()))
                        .child(
                            Button::new("remote-cwd-close")
                                .ghost().xsmall()
                                .icon(Icon::new(IconName::X))
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(RemoteDirectoryEvent::Close);
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .px_4()
                        .pt_4()
                        .pb_3()
                        .gap_3()
                        .child(div().text_sm()
                            .child("Start the agent in this Linux directory:"))
                        .child(
                            ui::field()
                                .child(Icon::new(IconName::Folder).small().text_color(ui::faint()))
                                .child(Input::new(&self.input).appearance(false).small()),
                        )
                        .child(div().text_xs().text_color(ui::muted())
                            .child("Type an absolute Linux path or select a shortcut below. Folders must already exist on the remote machine."))
                        .when_some(self.error.clone(), |column, error| {
                            column.child(div().text_xs().text_color(ui::danger())
                                .child(error))
                        }),
                )
                .child(
                    div()
                        .id("remote-cwd-shortcuts")
                        .min_h_0()
                        .max_h(px(280.))
                        .overflow_y_scroll()
                        .px_4()
                        .pb_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(ui::section_label("Known directories"))
                                .children(paths),
                        ),
                )
                .child(
                    h_flex()
                        .px_4()
                        .py_3()
                        .gap_2()
                        .border_t_1()
                        .border_color(ui::border())
                        .child(div().flex_1().text_xs().text_color(ui::faint())
                            .child("Only the new agent uses this directory."))
                        .child(
                            Button::new("remote-cwd-cancel")
                                .ghost().small().label("Cancel")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(RemoteDirectoryEvent::Close);
                                })),
                        )
                        .child(
                            Button::new("remote-cwd-launch")
                                .primary().small()
                                .icon(Icon::new(IconName::Plus))
                                .label("Create agent")
                                .on_click(cx.listener(|this, _, _, cx| this.launch(cx))),
                        ),
                ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::directory_suggestions;
    use crate::features::machines::model::{Snapshot, Terminal};

    #[test]
    fn shortcut_paths_are_machine_scoped_and_unique() {
        let snapshot = Snapshot {
            local_name: "thispc".into(),
            machines: Vec::new(),
            terminals: vec![
                Terminal {
                    address: "javipi/t1".into(), title: "".into(),
                    cwd: "/srv/workspaces/PoeTradePlus2".into(), state: "live".into(),
                },
                Terminal {
                    address: "javipi/t2".into(), title: "".into(),
                    cwd: "/srv/workspaces/PoeTradePlus2".into(), state: "live".into(),
                },
                Terminal {
                    address: "other/t3".into(), title: "".into(),
                    cwd: "/data/other".into(), state: "live".into(),
                },
                Terminal {
                    address: "javipi/t4".into(), title: "".into(),
                    cwd: r"C:\\Users\\test".into(), state: "live".into(),
                },
            ],
        };
        let pi = directory_suggestions("javipi", Some(&snapshot));
        assert_eq!(pi.first().map(String::as_str), Some("/srv/workspaces/JaviPi"));
        assert!(pi.iter().any(|path| path == "/srv/workspaces"));
        assert_eq!(pi.iter().filter(|path| path.as_str() == "/srv/workspaces/PoeTradePlus2").count(), 1);
        assert!(!pi.iter().any(|path| path == "/data/other"));
        assert!(!pi.iter().any(|path| path.contains("Users")));

        let other = directory_suggestions("other", Some(&snapshot));
        assert_eq!(other.first().map(String::as_str), Some("/"));
        assert!(other.iter().any(|path| path == "/data/other"));
        assert!(!other.iter().any(|path| path.contains("PoeTradePlus2")));
    }
}
