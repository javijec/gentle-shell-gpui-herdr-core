//! Machine registration and removal through the local Herdr Core API.
//! Sessions belong exclusively to the Machines navigator, never this modal.

pub(crate) mod model;
mod directory_picker;
mod view;

pub use directory_picker::{RemoteDirectoryEvent, RemoteDirectoryPicker};

use std::time::Duration;

use gpui_kit::*;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::InputState;
use gpui_kit::component::WindowExt as _;

pub enum MachinesEvent {
    Close,
    /// The registered machine list changed; refresh the workspace navigator.
    Changed,
}

pub struct MachinesView {
    snapshot: Option<model::Snapshot>,
    loading: bool,
    busy: bool,
    error: Option<String>,
    info: Option<String>,
    show_add_form: bool,
    /// true: custom command reaching a remote Herdr Core; false: host target.
    add_via: bool,
    add_host: Entity<InputState>,
    add_command: Entity<InputState>,
    add_name: Entity<InputState>,
    _load: Task<()>,
    _action: Task<()>,
    /// Only polls machine.list while this modal is alive.
    _poll: Task<()>,
}

impl EventEmitter<MachinesEvent> for MachinesView {}

impl MachinesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let add_host = cx.new(|cx| InputState::new(window, cx)
            .placeholder("Host or user@host[:port]"));
        let add_command = cx.new(|cx| InputState::new(window, cx)
            .placeholder("e.g. wsl.exe -d Ubuntu -e sh -c"));
        let add_name = cx.new(|cx| InputState::new(window, cx)
            .placeholder("Name for this machine in Herdr"));
        let mut view = Self {
            snapshot: None,
            loading: false,
            busy: false,
            error: None,
            info: None,
            show_add_form: false,
            add_via: true,
            add_host,
            add_command,
            add_name,
            _load: Task::ready(()),
            _action: Task::ready(()),
            _poll: Task::ready(()),
        };
        if crate::features::terminal::herdr_enabled() {
            view.refresh(cx);
            view._poll = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(10)).await;
                    let Ok(()) = this.update(cx, |this, cx| {
                        if !this.busy && !this.loading {
                            this.start_load(false, cx);
                        }
                    }) else {
                        break;
                    };
                }
            });
        }
        view
    }

    pub fn new_for_registration(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new(window, cx);
        view.begin_add_machine(window, cx);
        view
    }

    pub(crate) fn begin_add_machine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy {
            return;
        }
        self.show_add_form = true;
        self.error = None;
        self.info = None;
        self.add_name.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn set_add_via(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.add_via = enabled;
        self.error = None;
        if enabled {
            self.add_command.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.add_host.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn cancel_add_machine(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            self.show_add_form = false;
            self.error = None;
            cx.notify();
        }
    }

    fn submit_add_machine(&mut self, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled()
            || !self.show_add_form || self.busy
        {
            return;
        }
        let via = self.add_via;
        let connection = if via {
            self.add_command.read(cx).value().trim().to_owned()
        } else {
            self.add_host.read(cx).value().trim().to_owned()
        };
        let name = self.add_name.read(cx).value().trim().to_owned();
        if connection.is_empty() || (via && name.is_empty()) {
            self.error = Some(if via {
                "Enter a machine name and a Herdr connection command.".into()
            } else {
                "Enter a host or network target for Herdr.".into()
            });
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        self.info = Some("Adding machine to Herdr Core...".into());
        let job = cx.background_executor().spawn(async move {
            model::add_machine(&connection, (!name.is_empty()).then_some(name.as_str()), via)
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(()) => {
                        this.show_add_form = false;
                        this.info = Some("Machine registered in Herdr Core.".into());
                        cx.emit(MachinesEvent::Changed);
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// List only registered profiles, never terminal lists. Explicit refresh
    /// clears an old error; polling preserves any actionable operation error.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.start_load(true, cx);
    }

    fn start_load(&mut self, explicit: bool, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy {
            return;
        }
        self.loading = true;
        if explicit {
            self.error = None;
        }
        let job = cx.background_executor().spawn(async { model::discover_registered_machines() });
        self._load = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(snapshot) => {
                        // Only real profile or health changes trigger the
                        // workspace's existing navigator refresh path.
                        let changed = this.snapshot.as_ref().is_some_and(|old| {
                            old.local_name != snapshot.local_name
                                || old.machines != snapshot.machines
                        });
                        this.snapshot = Some(snapshot);
                        if changed {
                            cx.emit(MachinesEvent::Changed);
                        }
                    }
                    Err(error) => {
                        if explicit || this.snapshot.is_none() {
                            this.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// An immediate retry, not a new connection profile or a fresh terminal.
    /// Herdr Core continues retrying automatically if this attempt fails.
    fn reconnect_machine(&mut self, name: String, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy
            || self.snapshot.as_ref().is_none_or(|snapshot| {
                !snapshot.machines.iter().any(|machine| machine.name == name)
            })
        {
            return;
        }
        self.busy = true;
        self.error = None;
        self.info = Some(format!("Requesting connection to {name}..."));
        let job = cx.background_executor().spawn(async move {
            model::reconnect_machine(&name)
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(machine) => {
                        this.info = Some(format!(
                            "Herdr connection to {}: {}. Its automatic retry remains active.",
                            machine.name, machine.state,
                        ));
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Confirmation is required, including when a registered machine is
    /// disconnected. Forgetting it leaves its remote programs running.
    fn confirm_remove_machine(
        &mut self, name: String, window: &mut Window, cx: &mut Context<Self>,
    ) {
        if self.busy || self.snapshot.as_ref().is_none_or(|snapshot| {
            !snapshot.machines.iter().any(|machine| machine.name == name)
        }) {
            return;
        }
        let title = format!("Remove machine {name} from Herdr?");
        let description = format!(
            "This forgets the connection to {name} in Herdr Core. \
             Its remote agents, terminals and files are NOT deleted or stopped. \
             You can register the machine again later."
        );
        let weak = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let weak = weak.clone();
            let name = name.clone();
            alert
                .title(title.clone())
                .description(description.clone())
                .ok_text("Remove machine")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.remove_machine(name.clone(), cx);
                    });
                    true
                })
        });
    }

    fn remove_machine(&mut self, name: String, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.info = Some(format!("Removing {name} from Herdr..."));
        let job = cx.background_executor().spawn(async move {
            model::remove_machine(&name)
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(()) => {
                        this.info = Some("Machine removed. Remote sessions remain running.".into());
                        cx.emit(MachinesEvent::Changed);
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }
}
