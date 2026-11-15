//! Machines: discover the local Herdr Core's registered remote machines,
//! inspect persistent terminal addresses, and open existing/new shell panels.

pub(crate) mod model;
mod directory_picker;
mod view;

pub use directory_picker::{RemoteDirectoryEvent, RemoteDirectoryPicker};

use gpui_kit::*;
use gpui_kit::component::input::InputState;

pub enum MachinesEvent {
    Close,
    /// Attach the existing Herdr terminal address in a new GPUI tab.
    OpenTerminal(String),
    /// Explicitly launch a Pi agent on the local computer (None) or a
    /// connected registered remote machine (Some). Nothing auto-spawns.
    NewAgent(Option<String>),
}

pub struct MachinesView {
    snapshot: Option<model::Snapshot>,
    loading: bool,
    busy: bool,
    error: Option<String>,
    info: Option<String>,
    show_add_form: bool,
    add_target: Entity<InputState>,
    add_name: Entity<InputState>,
    _load: Task<()>,
    _action: Task<()>,
}

impl EventEmitter<MachinesEvent> for MachinesView {}

impl MachinesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let add_target = cx.new(|cx| InputState::new(window, cx)
            .placeholder("SSH Host or user@host[:port]"));
        let add_name = cx.new(|cx| InputState::new(window, cx)
            .placeholder("Optional display name"));
        let mut this = Self {
            snapshot: None,
            loading: false,
            busy: false,
            error: None,
            info: None,
            show_add_form: false,
            add_target,
            add_name,
            _load: Task::ready(()),
            _action: Task::ready(()),
        };
        // Defense in depth: even a programmatic view construction must never
        // connect to Herdr while the native/local backend is selected.
        if crate::features::terminal::herdr_enabled() {
            this.refresh(cx);
        }
        this
    }

    /// Open Machines directly on the registration form when the header +
    /// was used. This is never a new-agent operation.
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
        self.add_target.update(cx, |target, cx| target.focus(window, cx));
        cx.notify();
    }

    fn cancel_add_machine(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            self.show_add_form = false;
            self.error = None;
            cx.notify();
        }
    }

    /// Register an SSH machine in Herdr. No agent or terminal is launched.
    fn submit_add_machine(&mut self, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled()
            || !self.show_add_form || self.busy
        {
            return;
        }
        let target = self.add_target.read(cx).value().trim().to_owned();
        let name = self.add_name.read(cx).value().trim().to_owned();
        if target.is_empty() {
            self.error = Some("Enter an SSH target, for example dietpi@javipi.".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        self.info = Some(format!("Registering {target} in Herdr..."));
        let job = cx.background_executor().spawn(async move {
            model::add_machine(&target, (!name.is_empty()).then_some(name.as_str()))
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(()) => {
                        this.show_add_form = false;
                        this.info = Some("Machine added to Herdr Core.".into());
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Background discovery must never block UI painting or key handling.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy {
            return;
        }
        self.loading = true;
        self.error = None;
        let job = cx.background_executor().spawn(async { model::discover() });
        self._load = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(snapshot) => this.snapshot = Some(snapshot),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Open exactly this address; no terminal.run and no process duplication.
    fn open_terminal(&mut self, address: String, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() {
            return;
        }
        if self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.terminals.iter().any(|t| t.address == address && t.live())
        }) {
            cx.emit(MachinesEvent::OpenTerminal(address));
        }
    }

    /// 4C: the machine picker only emits an explicit new-agent choice.
    /// The workspace owns the tab and enforces isolation from other agents.
    fn new_agent(&mut self, machine: Option<String>, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy {
            return;
        }
        let available = match machine.as_deref() {
            None => true,
            Some(name) => self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.machines.iter().any(|m| m.name == name && m.connected())
            }),
        };
        if available {
            cx.emit(MachinesEvent::NewAgent(machine));
        } else {
            self.error = Some("This machine is not connected; agent not launched".into());
            cx.notify();
        }
    }

    /// All shells in Machines use Herdr Core. This action is unavailable
    /// under the native/local backend.
    fn new_shell(&mut self, machine: Option<String>, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.info = Some(match &machine {
            Some(name) => format!("Starting a shell on {name}..."),
            None => "Starting a local Herdr shell...".into(),
        });
        let job = cx.background_executor().spawn(async move {
            model::create_shell(machine.as_deref())
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(address) => cx.emit(MachinesEvent::OpenTerminal(address)),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Explicitly request a machine connection; do not SSH from GPUI itself.
    fn connect_machine(&mut self, name: String, cx: &mut Context<Self>) {
        if !crate::features::terminal::herdr_enabled() || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.info = Some(format!("Connecting to {name}..."));
        let job = cx.background_executor().spawn(async move {
            model::connect_machine(&name)
        });
        self._action = cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.info = None;
                match result {
                    Ok(()) => {
                        this.info = Some("Machine connection requested".into());
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
