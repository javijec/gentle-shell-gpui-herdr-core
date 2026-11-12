//! Machines: discover the local Herdr Core's registered remote machines,
//! inspect persistent terminal addresses, and open existing/new shell panels.

mod model;
mod view;

use gpui_kit::*;

pub enum MachinesEvent {
    Close,
    /// Attach the existing Herdr terminal address in a new GPUI tab.
    OpenTerminal(String),
}

pub struct MachinesView {
    snapshot: Option<model::Snapshot>,
    loading: bool,
    busy: bool,
    error: Option<String>,
    info: Option<String>,
    _load: Task<()>,
    _action: Task<()>,
}

impl EventEmitter<MachinesEvent> for MachinesView {}

impl MachinesView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            snapshot: None,
            loading: false,
            busy: false,
            error: None,
            info: None,
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
